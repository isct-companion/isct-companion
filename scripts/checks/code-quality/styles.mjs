import { parse as parseCss } from "postcss";
import { walk } from "./functions.mjs";

const number = String.raw`[+-]?(?:\d+(?:\.\d+)?|\.\d+)`;
const cssUnit = new RegExp(
  String.raw`(?<![\w.-])${number}(?:px|rem|em|ch|vw|vh|%|pt|ms|s)(?![\w-])`,
  "gi",
);
const cssColor =
  /#[\da-f]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|oklab|lch|oklch)\([^)]*\)|(?<![\w-])(?:black|white|gray|grey|red|green|blue|yellow|orange|purple)(?![\w-])/gi;

function sourceRole(property) {
  if (!property.startsWith("--")) return property;
  if (/radius/.test(property)) return "border-radius";
  if (/padding|margin|space|gap|gutter/.test(property)) return "padding";
  if (/font-size|type-.*-size/.test(property)) return "font-size";
  if (/shadow/.test(property)) return "box-shadow";
  if (/duration|transition|motion/.test(property)) return "transition";
  if (/color|surface|accent|ink|muted|line|(?:^|-)bg\b/.test(property))
    return "color";
  return "";
}

function cssSignals(property, value, tokenNames, emit, inline = false) {
  const role = sourceRole(property);
  const alias = /^var\(\s*(--[\w-]+)\s*\)$/.exec(value.trim());
  if (property.startsWith("--") && alias && tokenNames.has(alias[1])) {
    emit("Token alias", property, alias[1]);
  }
  const cleaned = value.replace(/url\([^)]*\)/gi, "");
  if (
    role &&
    /color|background|border|outline|shadow|^(?:fill|stroke)$/.test(role)
  ) {
    for (const match of cleaned.matchAll(cssColor)) {
      if (
        /^[a-z]+$/i.test(match[0]) ||
        !match[0].startsWith("#") ||
        [4, 5, 7, 9].includes(match[0].length)
      ) {
        emit("Color", property, match[0]);
      }
    }
  }
  const measures = [...cleaned.matchAll(cssUnit)].map((match) => match[0]);
  if (inline && new RegExp(`^${number}$`).test(cleaned))
    measures.push(`${cleaned}px`);
  for (const literal of new Set(measures)) {
    if (parseFloat(literal) === 0) continue;
    if (/radius/.test(role) && !["50%", "999px"].includes(literal))
      emit("Radius", property, literal);
    if (/^(?:padding|margin|gap|row-gap|column-gap)(?:-|$)/.test(role))
      emit("Spacing", property, literal);
    if (/^font(?:-size)?$/.test(role)) emit("Font size", property, literal);
    if (/shadow/.test(role)) emit("Shadow", property, literal);
    if (
      /^(?:transition|animation)(?:-|$)/.test(role) &&
      /(?:ms|s)$/i.test(literal)
    )
      emit("Motion", property, literal);
  }
}

// Keep concrete branches in inline styles while leaving runtime style calculations alone.
function* literalBranches(node) {
  if (!node) return;
  if (node.type === "StringLiteral" || node.type === "NumericLiteral")
    yield node;
  if (
    node.type === "UnaryExpression" &&
    node.operator === "-" &&
    node.argument.type === "NumericLiteral"
  ) {
    yield { value: -node.argument.value, loc: node.loc };
  }
  if (node.type === "LogicalExpression") {
    if (node.operator !== "&&") yield* literalBranches(node.left);
    yield* literalBranches(node.right);
  }
  if (node.type === "ConditionalExpression") {
    yield* literalBranches(node.consequent);
    yield* literalBranches(node.alternate);
  }
  if (node.type === "TSAsExpression" || node.type === "TSSatisfiesExpression")
    yield* literalBranches(node.expression);
}

function* styleObjects(node) {
  if (!node) return;
  if (node.type === "ObjectExpression") yield node;
  if (node.type === "LogicalExpression") {
    yield* styleObjects(node.left);
    yield* styleObjects(node.right);
  }
  if (node.type === "ConditionalExpression") {
    yield* styleObjects(node.consequent);
    yield* styleObjects(node.alternate);
  }
  if (node.type === "TSAsExpression" || node.type === "TSSatisfiesExpression")
    yield* styleObjects(node.expression);
}

// Native matches cover direct numeric arguments to common UI APIs. Mask comments
// without moving source offsets, including calls split across multiple lines.
const nativeComments =
  /"""[\s\S]*?"""|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\/\*[\s\S]*?\*\/|\/\/[^\n]*/g;
const nativeColors =
  "black|white|red|blue|green|gray|grey|orange|yellow|purple|pink|brown|cyan|magenta|indigo|mint|teal";
const swiftMeasures = [
  [
    "Spacing",
    String.raw`\b(?:padding|spacing|EdgeInsets)\(\s*(?:\.[a-zA-Z]+\s*,\s*)?|\b(?:spacing|top|leading|bottom|trailing):\s*`,
  ],
  ["Radius", String.raw`\bcornerRadius(?:\(\s*|:\s*)`],
  [
    "Font size",
    String.raw`\b(?:system|custom)\(\s*(?:"[^"\n]+"\s*,\s*)?(?:fixedSize|size):\s*`,
  ],
  ["Shadow", String.raw`\bshadow\(\s*(?:color:\s*[^,()]+,\s*)?radius:\s*`],
  [
    "Motion",
    String.raw`\b(?:easeInOut|easeIn|easeOut|linear|spring|smooth|snappy|bouncy)\(\s*duration:\s*`,
  ],
];
const kotlinMeasures = [
  [
    "Spacing",
    String.raw`\b(?:padding|spacedBy|PaddingValues)\((?:[^()]*?,\s*|\s*)(?:(?:start|top|end|bottom|horizontal|vertical|all)\s*=\s*)?`,
    "dp",
  ],
  [
    "Radius",
    String.raw`\b(?:RoundedCornerShape|CornerSize)\((?:[^()]*?,\s*|\s*)(?:\w+\s*=\s*)?`,
    "dp",
  ],
  [
    "Shadow",
    String.raw`\b(?:shadow|tonalElevation|shadowElevation)(?:\(\s*|\s*=\s*)(?:elevation\s*=\s*)?`,
    "dp",
  ],
  [
    "Motion",
    String.raw`\b(?:tween|keyframes)\(\s*(?:durationMillis\s*=\s*)?|\bdurationMillis\s*=\s*`,
  ],
];

function nativeSignals(source, kotlin, emit, reference) {
  const code = source.replace(nativeComments, (text) =>
    text.startsWith("/") ? text.replace(/[^\n]/g, " ") : text,
  );
  const starts = [
    0,
    ...Array.from(code.matchAll(/\n/g), (match) => match.index + 1),
  ];
  function at(rule, property, value, offset) {
    let low = 0;
    let high = starts.length;
    while (low + 1 < high) {
      const mid = Math.floor((low + high) / 2);
      if (starts[mid] <= offset) low = mid;
      else high = mid;
    }
    emit(low + 1, rule, property, value);
  }
  const color = kotlin
    ? new RegExp(
        String.raw`\bColor\.(?:${nativeColors})\b|\bColor\(\s*0x[\da-f_]+(?:u?l)?\b`,
        "gi",
      )
    : new RegExp(
        String.raw`\b(?:Color|UIColor)\.(?:${nativeColors})\b|\b(?:foregroundStyle|foregroundColor|background|tint|fill|stroke)\(\s*\.(?:${nativeColors})\b|\b(?:Color|UIColor)\(\s*(?:(?:\.sRGB(?:Linear)?|\.displayP3)\s*,\s*)?(?:red|white|hue):\s*${number}|\bColor\(\s*hex:\s*"#?[\da-f]{6,8}"`,
        "gi",
      );
  for (const match of code.matchAll(color))
    at("Color", "Color", match[0].replace(/\s+/g, " "), match.index);
  for (const match of code.matchAll(
    /\b(?:Theme|IsctTokens(?:Dark)?)\.(?:colors\.)?\w+/g,
  ))
    reference(match[0]);
  const measures = kotlin ? kotlinMeasures : swiftMeasures;
  for (const [rule, prefix, unit] of measures) {
    const suffix = unit ? String.raw`\.${unit}` : "";
    const pattern = new RegExp(
      String.raw`(?:${prefix})(${number}${suffix})(?=\s*[,)]|\s*$)`,
      "gm",
    );
    for (const match of code.matchAll(pattern)) {
      // Compose calls can specify several different edges in one argument list.
      const literals = unit
        ? [
            ...match[0].matchAll(
              new RegExp(String.raw`(?<![\w.])${number}\.${unit}\b`, "g"),
            ),
          ]
        : [];
      if (literals.length === 0)
        literals.push({ 0: match[1], index: match[0].lastIndexOf(match[1]) });
      const property = /^\w+/.exec(match[0])?.[0] ?? rule;
      for (const literal of literals) {
        if (parseFloat(literal[0]) !== 0)
          at(rule, property, literal[0], match.index + literal.index);
      }
    }
  }
  if (kotlin) {
    for (const match of code.matchAll(
      new RegExp(String.raw`(?<![\w.])${number}\.sp\b`, "g"),
    )) {
      if (parseFloat(match[0]) !== 0)
        at("Font size", "sp", match[0], match.index);
    }
  }
}

function readTokenNames(read) {
  const tokenNames = new Set();
  for (const name of ["tokens.css", "tokens.dark.css", "themes.css"]) {
    parseCss(read(`apps/desktop/src/generated/design/${name}`)).walkDecls(
      /^--/,
      (decl) => tokenNames.add(decl.prop),
    );
  }
  return tokenNames;
}

function cssReferences(value, tokenNames, reference) {
  for (const [, name] of value.matchAll(/\bvar\(\s*(--[\w-]+)/g)) {
    if (tokenNames.has(name)) reference(name);
  }
}

function scanStyles(file, source, tokenNames, emit, reference) {
  if (file.endsWith(".css")) {
    parseCss(source, { from: file }).walkDecls((decl) => {
      cssReferences(decl.value, tokenNames, reference);
      cssSignals(decl.prop, decl.value, tokenNames, (...signal) =>
        emit(decl.source.start.line, ...signal),
      );
    });
  } else {
    nativeSignals(source, file.endsWith(".kt"), emit, reference);
  }
}

function scanInlineStyle(node, tokenNames, emit, reference) {
  if (node.type !== "JSXAttribute" || node.name.name !== "style") return;
  for (const object of styleObjects(node.value?.expression)) {
    for (const prop of object.properties) {
      if (prop.type !== "ObjectProperty" || prop.computed) continue;
      const raw = prop.key.name ?? prop.key.value;
      if (typeof raw !== "string") continue;
      const property = raw.startsWith("--")
        ? raw
        : raw.replace(/[A-Z]/g, (char) => `-${char.toLowerCase()}`);
      for (const literal of literalBranches(prop.value)) {
        if (typeof literal.value === "string")
          cssReferences(literal.value, tokenNames, reference);
        cssSignals(
          property,
          String(literal.value),
          tokenNames,
          (...signal) => emit(literal.loc.start.line, ...signal),
          typeof literal.value === "number" && !raw.startsWith("--"),
        );
      }
    }
  }
}

// Groups of one value under one role that appear in more than one file.
function repeatedStyles(findings) {
  const repeats = new Map();
  for (const finding of findings) {
    const key = JSON.stringify([
      finding.platform,
      finding.rule,
      finding.property,
      finding.value.toLowerCase(),
    ]);
    if (!repeats.has(key)) repeats.set(key, []);
    repeats.get(key).push(finding);
  }
  return [...repeats.values()]
    .filter((group) => new Set(group.map((hit) => hit.file)).size > 1)
    .sort((a, b) => b.length - a.length);
}

export function createStyleSection({ read }) {
  const tokenNames = readTokenNames(read);
  const findings = [];
  const references = new Map();
  return {
    accepts: (file) => /\.(?:css|tsx|kt|swift)$/.test(file),
    visit({ file, platform, source, lines, ast }) {
      const seen = new Set();
      const reference = (name) => {
        const key = `${platform}/${name}`;
        if (!references.has(key))
          references.set(key, { platform, name, count: 0, files: new Set() });
        const ref = references.get(key);
        ref.count++;
        ref.files.add(file);
      };
      const emit = (line, rule, property, value) => {
        const key = JSON.stringify([line, rule, property, value]);
        if (seen.has(key)) return;
        seen.add(key);
        findings.push({
          file,
          line,
          platform,
          rule,
          property,
          value,
          code: lines[line - 1].trim().slice(0, 220),
        });
      };
      if (file.endsWith(".tsx"))
        walk(ast(), (node) =>
          scanInlineStyle(node, tokenNames, emit, reference),
        );
      else scanStyles(file, source, tokenNames, emit, reference);
    },
    finish() {
      findings.sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line);
      const tokenReferences = [...references.values()]
        .sort((a, b) => b.count - a.count)
        .map((ref) => ({ ...ref, files: [...ref.files] }));
      return {
        data: { findings, repeated: repeatedStyles(findings), tokenReferences },
        errors: [],
        skipped: [],
      };
    },
  };
}
