"""Rust and Swift function metrics for scan.mjs.

Reads a JSON list of absolute paths on stdin and prints one JSON object per
line: a function row, {"file", "error"} for a file that did not parse, or
{"unavailable"} when the Tree-sitter packages are not installed.
"""

import json
from pathlib import Path
import sys

try:
    from tree_sitter import Language, Parser
    import tree_sitter_rust
    import tree_sitter_swift
except ImportError as error:
    print(json.dumps({'unavailable': f'{error.name} is not installed for {sys.executable}. '
                      'Install requirements.txt there or pass --python.'}))
    sys.exit(0)

PARSERS = {
    '.rs': Parser(Language(tree_sitter_rust.language())),
    '.swift': Parser(Language(tree_sitter_swift.language())),
}
RUST_BRANCHES = {'if_expression', 'for_expression', 'while_expression', 'loop_expression'}
SWIFT_BRANCHES = {
    'if_statement', 'guard_statement', 'for_statement', 'while_statement', 'repeat_while_statement',
    'ternary_expression', 'catch_block',
}


def signature(node, rust):
    if rust:
        if node.type != 'function_item':
            return None
        name = node.child_by_field_name('name').text.decode()
        params = node.child_by_field_name('parameters').named_children
        count = sum(p.type in {'parameter', 'variadic_parameter'} for p in params)
        return name, count, node.child_by_field_name('body')

    names = {'init_declaration': 'init', 'deinit_declaration': 'deinit', 'subscript_declaration': 'subscript'}
    if node.type in {'function_declaration', 'property_declaration'}:
        declared = node.child_by_field_name('name')
        name = declared.text.decode() if declared else None
    else:
        name = names.get(node.type)
    if name is None:
        return None
    # Stored properties have neither field and are skipped.
    body = node.child_by_field_name('body') or node.child_by_field_name('computed_value')
    if node.type == 'subscript_declaration':
        body = next((child for child in node.named_children if child.type == 'computed_property'), None)
    params = sum(child.type == 'parameter' for child in node.named_children)
    return (name, params, body) if body is not None else None


def is_else_if(node, rust):
    """`else if` continues its chain at the same depth, as in scan.mjs."""
    if rust:
        return node.type == 'if_expression' and node.parent.type == 'else_clause'
    return node.type == 'if_statement' and node.parent.type == 'if_statement'


def metrics(body, rust):
    controls = RUST_BRANCHES if rust else SWIFT_BRANCHES
    multiway = 'match_expression' if rust else 'switch_statement'
    arm = 'match_arm' if rust else 'switch_entry'
    branches = nesting = 0
    pending = [(body, 0)]
    while pending:
        node, depth = pending.pop()
        # Named nested declarations have their own row. Inline closures stay with their caller.
        if node != body and signature(node, rust):
            continue
        if node.type in controls or node.type == arm:
            branches += 1
        if node.type == multiway or (node.type in controls and not is_else_if(node, rust)):
            depth += 1
            nesting = max(nesting, depth)
        pending.extend((child, depth) for child in node.named_children)
    return branches, nesting


def scan(file):
    path = Path(file)
    rust = path.suffix == '.rs'
    root = PARSERS[path.suffix].parse(path.read_bytes()).root_node
    if root.has_error:
        yield {'file': file, 'error': 'Tree-sitter syntax error. Function metrics omitted for this file.'}
        return
    pending = [root]
    while pending:
        node = pending.pop()
        pending.extend(reversed(node.named_children))
        declared = signature(node, rust)
        if declared is None:
            continue
        name, parameters, body = declared
        branches, nesting = metrics(body, rust)
        yield dict(file=file, name=name, line=node.start_point.row + 1, endLine=node.end_point.row + 1,
                   parameters=parameters, branches=branches, nesting=nesting)


for file in json.load(sys.stdin):
    try:
        records = list(scan(file))
    except Exception as error:  # one unreadable file must not drop the others
        records = [{'file': file, 'error': f'{type(error).__name__}: {error}'}]
    for record in records:
        print(json.dumps(record))
