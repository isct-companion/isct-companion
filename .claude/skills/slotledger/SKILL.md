---
name: slotledger
description: When the project has the slotledger MCP (record/handover/ask/respond/brief/split/archive tools), use it for lasting decisions other work rests on and for handovers the user asks for, instead of PLAN/TODO/HANDOFF documents. Use it when making or changing such a decision, noticing a premise has changed, writing, updating or closing a handover, retiring decisions, or when the user answers numbered items of the end-of-turn report. Demos, smoke tests, exploratory runs and one-off checks need no records.
---

# Recording project state with slotledger

slotledger stores lasting decisions as versioned slots. Each decision states its rationale and premises; when a premise gets a new value, the decisions that rest on it directly are flagged automatically. Unfinished work the user hands over goes in a short handover that is deleted when the work ends. It won't block you; just follow the steps below.

## Setting up

Before installing slotledger into a project (or when brief says the settings aren't set), ask the user two things, then pass the answers to `slotledger init` or `slotledger config` (brief prints the exact command):

1. **Language**: which language decisions, rationales, handover titles and questions should be written in. slotledger's own messages stay in English.
2. **Strictness**:
   - strict: record every lasting decision as soon as you make it.
   - balanced (default): record decisions other work or a later session rests on, and an assumption only when several decisions rest on it and it could turn out wrong.
   - loose: only goals and the main decisions; The end-of-turn check never interrupts you.

Follow the language and strictness that brief shows for the project.

For Codex, install with `slotledger init --client codex` (or `--client both` for Codex and Claude Code). This writes project-local MCP and hooks config and installs this skill under `.agents/skills/slotledger/`. Codex must trust the project and its hook definitions before the hooks run.

Call `brief` before editing when hooks are unavailable; this establishes the session's git baseline.

## When to record

Record only lasting decisions that later work must follow and that can be stated in engineering terms. The test: if this changed later, would some other decision or session go wrong without being told? If not, don't record it. Record a decision as soon as you make or change it, not at the end. Keep recording light: say what was decided and why, in plain words. You don't classify decisions: slotledger's compile step sorts them into tables for people later.

What counts: a behavior rule (in this situation, do or don't do this), structure or ownership, configuration and parameters (including an experiment's setup), a definition of a term or metric, an acceptance threshold, an outside limit. In research projects too, only what can be defined this way goes in: when a result changes the plan, record the engineering decision it leads to ("main method is now A2, configured as …"), not the result.

**A new decision** gets a short title (what it's about, a few words, at most 40 characters), a value and a rationale. The tool assigns its id, like `k7m2qx`, and returns it:

```
record({ title: "Sync backend", value: "Self-hosted server", rationale: "..." })  →  Recorded k7m2qx v1 "Sync backend"
```

**To change one**, pass its id with `update: true`. An id without `update: true` is refused, so a new decision never overwrites an old one by accident. A new title alone is also an update, and asks nothing of what rests on it. Older keys such as `approach/sync-backend` or `goal/offline` work as ids too.

Don't record: results or measurements, hypotheses, plans, steps or progress, receipts of one run or release (commits, hashes, reservation windows, versions frozen for a run), debugging findings, one-off work, demos, smoke tests, exploratory runs, or implementation details the code already shows (which library, file or function). These belong in the project's own reports or in the code. A value that reads like a one-off record (a hash, a date or time window, 本轮 / 第 N 步 / 已完成, an interval, one release) is held once: restate it without the one-time details, or pass `lasting: true` if it really is a lasting decision.

Keep it to few calls: a decision the user states is one `record` with their words in userQuote, and the reason they give goes in its rationale, not in a separate decision (add one only when several decisions rest on it); there's no need to search with `brief` first, since `record` holds back near-duplicates itself.

- **Write a clear rationale.** It is used to question the user when a premise changes.
- **Fill in depends** with the ids of the decisions this one rests on. The record result suggests candidates.
- **If the user said it, include userQuote.** Only then does it count as the user's decision.
- **Changing or retiring a decision the user made is refused.** Ask the user first; once they agree, write it with userQuote.
- **Update, don't duplicate.** A new decision that looks like an existing one is held back once with the similar ones listed (id, title, value): update one of them, merge with `replaces: [ids]`, or pass `new: true` if it really is separate.
- **Merge duplicates** when brief lists them: record the one to keep with `replaces`. The old ones are retired and what depended on them is re-pointed, keeping its status.
- **One decision, one thing that can change on its own.** If a value bundles several choices (say a loss's first-token term, its later-position weight and its teacher) and different decisions rest on different parts, split it with `split`: give the pieces titles (the tool assigns their ids), and in `dependents` point each decision at the pieces it really rests on, by piece title. Split several decisions in one call. Re-pointing asks nothing of anyone, and afterwards a change to one piece only flags what rests on that piece. `record` suggests this when most of a change's dependents turn out unaffected.

## When a premise changes

Don't stop, and don't quietly switch approaches in the code.

1. Record the new fact first, e.g. update the users' OS decision to macOS. The result lists the decisions that rest on it directly, and any handovers written against the old value.
2. In the same call, or one follow-up `record({ id, unaffected: [{ key: id, why }, ...] })`, name every decision the change doesn't affect. They stay as they are, and nothing further down is touched.
3. Adjust the work that depends only on the fact itself as usual.
4. The user's own decisions go into the end-of-turn report automatically for the user to decide. Until the user answers, don't change the work behind those decisions.
5. Review your own decisions yourself: if one still holds, update it with the same value. That re-locks its premises without a new version and keeps whoever decided it. If not, change it.
6. For each handover the result names, check it still holds; if not, rewrite its context, limits or steps.

Only a new value (or retiring, deleting, rejecting) flags anything, and only one level down: a confirmed decision doesn't ripple further. Handle reviews in a few batched calls yourself; don't hand each one to a subagent.

**When a premise is gone** (deleted, or retired with nothing replacing it), re-confirming what rests on it is refused, because that would drop it silently. brief and the end-of-turn reminder show the whole chain resting on it. For each thing directly on it, decide:
- it stands without it: `record({ id: <gone id>, unaffected: [{ key: id, why }, ...] })` drops the dependency, all in one call;
- it only existed because of it: `archive({ decisions, note })` retires it, then decide the same for what rested on that.

## Handovers

A handover carries unfinished work to a later session or someone else. Write one **only when the user says this is a handover**, with their words in userQuote, and only with what's left (at least one step still to do, or a note). Nothing else becomes a handover: work you finish in this session, or a request to do something now ("run the tests"), needs nothing recorded, and a rule the user wants kept from now on is a decision, not a handover. A handover is a short work order and holds only:

- **title**
- **context**: the decisions it relies on, each as `{ key: id, quote }` with the words that matter copied from the decision's current value. Look them up when you write the handover (`brief({ query })`); it then stands on its own. `context: []` if it relies on none. If a quoted decision later changes, brief and `task({ slug })` show it as changed since written.
- **limits**: limits for this work only (use at most 2 nodes, no new dependencies), at most 5 and each short. A limit may follow from a lasting decision or preference (`from`), but it stays one-off and ends with the handover; don't record it as a constraint. A limit that holds beyond this task is a constraint or decision of its own.
- **steps**: at most 20 short steps. Mark them done with `done: [step numbers]`.
- **note**: one short note (at most 500 characters) on where it stands and what's next, replaced on each update. Write it so whoever picks it up can continue. Progress logs belong in the project's own logs or reports.

`handover({ slug })` alone shows one. When the work ends, close it with `handover({ slug, close: "done" | "dropped", note })`: it's deleted and one line stays in the log. Handovers live in `.project/tasks/`, out of git; read and write them only through `handover` and `brief`, not the files.

## Compiling into tables

`compile` organizes the recorded decisions into tables people check, and rewrites each decision as a short dev-doc entry. **Call it only when the user asks** to compile or organize the decisions (often in a session opened for just that); never on your own, and not as part of other work. brief only notes how many decisions aren't in the tables yet.

1. `compile()` hands out a batch of up to 20 decisions, the current tables, the tables most likely to take them with their declared names, and the answer format.
2. Answer with `compile({ batch, answer })`: file each decision in a table (creating or renaming tables when needed), give it a status, a title and a dev-doc body, and its statements, each citing the sentences it comes from. Use only declared names for objects and conditions; declare a new one with a one-line meaning only for something new.
3. Each answer is checked before anything is written. On errors, fix them and answer the same batch again in full. A good answer is applied and the next batch comes back; keep going until nothing is left. Do the batches yourself rather than through subagents.

Afterwards `export` gives the tables as a document, and the web view shows them.

## Questions and replies

- If the user is around, ask them directly; record the answer with `respond`, passing the question ID from brief as question.
- If the user isn't around, use `ask`; the question appears in the end-of-turn report.
- Only when the user answers the numbered items of the end-of-turn report, e.g. "1A" or "withdraw 2", first call `brief({ prompt: "<their reply>" })` to see what the numbers mean, then record it with `respond`. userQuote is the user's exact words.

## Shared team docs

The team may follow the project in a shared doc (Notion, a wiki, or similar) that you write through its connector. slotledger stays the source of truth; the doc is a copy.

- **Writing:** call `export` (markdown for pages, json for databases) and write its output as is. Replace the whole page, or each entry under its heading (decision id, question ID, handover slug). Don't summarize, reword, or add your own entries; if something is missing, record it in slotledger first, then export again.
- **Reading:** when the doc has a teammate's edit or comment, record it in slotledger before you act on it:
  - a decision someone stated → `record` with their words in userQuote (changing the user's decisions still needs their agreement)
  - an answer to a listed question → `respond` with the question ID
  - anything that needs the user's call → ask the user, or `ask`
- After recording, export again and overwrite the doc, so their edit shows up as slotledger's version.

## Wrapping up

With lifecycle hooks, slotledger builds the end-of-turn report itself and shows it to the user, so don't repeat it. If it reminds you of a decision other work rests on that you missed, record it; if it's only an execution detail, just finish.

Without lifecycle hooks, call `checkpoint` before finishing each turn. If it returns a nudge, address any missing records and call `checkpoint({ continued: true })` to get the report. Show that report verbatim to the user; "No new findings" needs no mention. This fallback relies on you calling the tools; it cannot automatically continue a turn or display a report outside your response.

The user can see every project on this machine in a browser with `slotledger web`, and act there too. Brief tags what a person did: `human-edit` (they rewrote it in the web view), `human-approved` (they approved your proposal), `human-rejected` (they rejected it), `human-decided` (they decided it some other way). Treat tagged decisions as the user's: ask before changing them. A decision the user deleted simply isn't there any more; don't try to recover it.
