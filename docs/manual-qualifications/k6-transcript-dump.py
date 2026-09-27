#!/usr/bin/env python3
"""Read-only dump of an admitted edit session's transcript, for the K6 review.

Usage:
  python k6-transcript-dump.py --state-dir <dir> [--span PATH:START-END ...] [--expect LITERAL ...]

`--state-dir` is the directory that holds `journal.sqlite` (or the task directory
that holds a `state/` folder with it). The journal is opened through SQLite's
read-only, immutable URI, so this script cannot write to it and never touches
Shuttle, the workspace or the worker.

It prints what the edit model was shown and did, and answers, informationally,
the transcript criteria K6 names in S034: whether the target lines of a file were
covered by a read or find, and whether given literals (for example the
`package.json` script names) appeared anywhere in what the model saw. It reaches
no verdict; the operator decides whether the run succeeded.

The "transcript" is the first turn's user JSON (the previews of allowed and
reference files) plus every read or find observation the model received.
"""
import argparse
import json
import os
import re
import sqlite3
import sys


def esc(text, limit=300):
    """One printable line: controls and invisible characters are escaped, then capped."""
    out = []
    for ch in str(text):
        code = ord(ch)
        if ch == '\n':
            out.append('\\n')
        elif ch == '\t':
            out.append('\\t')
        elif code < 0x20 or 0x7F <= code <= 0x9F or code in (0x200B, 0x200C, 0x200D, 0x2028, 0x2029, 0xFEFF) \
                or 0x202A <= code <= 0x202E or 0x2066 <= code <= 0x2069:
            out.append('\\u{%x}' % code)
        else:
            out.append(ch)
    line = ''.join(out)
    return line if len(line) <= limit else line[:limit] + '...(cut)'


def open_journal(state_dir):
    for candidate in (os.path.join(state_dir, 'journal.sqlite'), os.path.join(state_dir, 'state', 'journal.sqlite')):
        if os.path.isfile(candidate):
            uri = 'file:' + os.path.abspath(candidate).replace('\\', '/') + '?mode=ro&immutable=1'
            return sqlite3.connect(uri, uri=True), candidate
    sys.exit(f'no journal.sqlite under {state_dir}')


def load(value):
    if value is None:
        return None
    return json.loads(value if isinstance(value, str) else value.decode('utf-8'))


def parse_span(text):
    match = re.fullmatch(r'(.+):(\d+)-(\d+)', text)
    if not match:
        sys.exit(f'--span must look like PATH:START-END (lines), got {text!r}')
    return match.group(1), int(match.group(2)), int(match.group(3))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--state-dir', required=True)
    parser.add_argument('--span', action='append', default=[], help='PATH:START-END (1-based lines) to check coverage of')
    parser.add_argument('--expect', action='append', default=[], help='a literal to look for in everything the model saw')
    args = parser.parse_args()
    spans = [parse_span(s) for s in args.span]

    conn, path = open_journal(args.state_dir)
    print(f'journal: {path} (opened read-only)')

    session = conn.execute('SELECT id, definition_json, terminal_reason, read_count, read_bytes FROM admitted_edit_sessions '
                           'ORDER BY rowid DESC LIMIT 1').fetchone()
    if not session:
        sys.exit('no edit session is recorded in this journal')
    session_id, definition_raw, terminal, read_count, read_bytes = session
    definition = load(definition_raw)
    revision = definition.get('context_revision') or 1
    allowed = definition['permission']['allowed_paths']
    print(f"\n== session {session_id[:12]}  context revision {revision}  "
          f"{'open' if terminal is None else 'closed: ' + esc(terminal, 120)}")
    print(f"   allowed paths: {allowed}   reads used {read_count} of 4, read bytes {read_bytes} of 6144")
    print(f"   profile digest: {definition['profile_digest'][:12]}...")
    files = definition['initial_context']['files']
    shown = [f for f in files if f.get('utf8_preview') and f['path'].replace('\\', '/') not in allowed]
    if revision == 2:
        summary = definition.get('plan_summary')
        state = 'dropped' if summary is None else ('cut' if definition.get('plan_summary_truncated') else 'included')
        print(f"   reference files shown: {len(shown)}, omitted: {definition.get('reference_omitted')}, plan summary: {state}")
        for f in shown:
            print(f"     reference {esc(f['path'])} ({f['kind']}, {f['bytes']} B, preview {len(f['utf8_preview'])} B)")
    else:
        print('   revision 1 session: no reference files or plan note were shown to the model')

    # Everything the model saw, as (where, text) pairs.
    seen = []
    for f in files:
        if f['path'].replace('\\', '/') in allowed and f.get('utf8_preview'):
            seen.append((f"preview of {f['path']}", f['utf8_preview']))
        elif revision == 2 and f.get('utf8_preview'):
            seen.append((f"reference preview of {f['path']}", f['utf8_preview']))
    if revision == 2 and definition.get('plan_summary'):
        seen.append(('plan note', definition['plan_summary']))

    turns = conn.execute('SELECT t.turn_index, t.request_id FROM admitted_edit_turns t WHERE t.session_id = ? '
                         'ORDER BY t.turn_index', (session_id,)).fetchall()
    covered = {}  # path -> list of (start_line, end_line, partial_last_line, turn)
    print('\n== turns')
    patch = None
    for index, request_id in turns:
        row = conn.execute('SELECT state, application, result_json, intent_json FROM model_requests WHERE id = ?',
                           (request_id,)).fetchone()
        state, application, result_raw, intent_raw = row
        result = load(result_raw) or {}
        decision = (result.get('reply') or {}).get('decision') or {}
        kind = next(iter(decision), None)
        body = decision.get(kind) if kind else None
        line = f"  turn {index}: {state}"
        if kind == 'AdmittedTextRead' and body:
            what = (f"{body['tool']} {esc(body['path'])} from line {body.get('start_line')} x {body.get('line_count')}"
                    if body['tool'] == 'read_task_text' else f"{body['tool']} {esc(body['path'])} literal {esc(body.get('literal'))!r}")
            line += f"  ->  {what}"
        elif kind == 'AdmittedTextPatch' and body:
            line += f"  ->  patch of {len(body['files'])} file(s)"
            patch = body
        else:
            line += f"  ->  {esc(application or kind or 'no reply recorded', 90)}"
        print(line)
        if index == 0:
            request = load(intent_raw).get('serialized_request') or {}
            post = next((e for e in request.get('exchanges', []) if e.get('method') == 'POST'), None)
            if post:
                messages = json.loads(post['body'])['messages']
                prompt = json.loads(messages[1]['content'])
                keys = sorted(prompt.keys())
                print(f"     turn-0 user JSON keys: {keys}")
                tools = [t['function']['name'] for t in json.loads(post['body']).get('tools', [])]
                print(f"     tools offered on turn 0: {tools}")
        obs = conn.execute('SELECT observation_json FROM admitted_edit_observations WHERE request_id = ?',
                           (request_id,)).fetchone()
        if obs:
            observation = load(obs[0])
            operation = observation['operation']
            result = observation['result']
            for excerpt in result.get('excerpts', []):
                covered.setdefault(operation['path'], []).append(
                    (excerpt['start_line'], excerpt['end_line'], excerpt.get('partial_last_line', False), index))
                seen.append((f"turn {index} read of {operation['path']} lines {excerpt['start_line']}-{excerpt['end_line']}",
                             excerpt['exact_utf8']))
                print(f"     lines {excerpt['start_line']}-{excerpt['end_line']}, bytes {excerpt['start_byte']}-{excerpt['end_byte']}"
                      f"{', last line partial' if excerpt.get('partial_last_line') else ''}"
                      f"{', truncated' if excerpt.get('truncated') else ''}")
            for match in result.get('matches', []):
                print(f"     match at bytes {match.get('start_byte')}-{match.get('end_byte')}")
            if operation.get('tool') == 'find_task_text' and not result.get('matches'):
                print('     (no matches)')

    print('\n== the patch')
    if patch is None:
        print('  no patch was recorded (the session ended without one)')
    else:
        for file in patch['files']:
            print(f"  {esc(file['path'])}: {len(file['hunks'])} hunk(s)")
            for number, hunk in enumerate(file['hunks'], 1):
                print(f"    hunk {number}: old {len(hunk['old_utf8'])} B  new {len(hunk['new_utf8'])} B")
                print(f"      - {esc(hunk['old_utf8'])}")
                print(f"      + {esc(hunk['new_utf8'])}")
        added = sorted({m for f in patch['files'] for h in f['hunks'] for m in re.findall(r'npm run ([\w:.-]+)', h['new_utf8'])})
        if added:
            print(f'  npm scripts named in the new text (informational; the doc-scripts-exist check is the real test): {added}')

    print('\n== transcript criteria (informational; no verdict)')
    for path, start, end in spans:
        wanted = set(range(start, end + 1))
        got = {}
        for s, e, partial, turn in covered.get(path, []):
            for line in range(s, e + 1):
                if line in wanted:
                    # the last line of a partial excerpt may be cut short
                    got.setdefault(line, set()).add((turn, partial and line == e))
        full = sorted(l for l, marks in got.items() if any(not p for _, p in marks))
        partial_only = sorted(l for l in got if l not in full)
        missing = sorted(wanted - set(got))
        turns_used = sorted({t for marks in got.values() for t, _ in marks})
        status = 'COVERED' if not missing and not partial_only else ('PARTLY' if got else 'NOT covered')
        print(f"  [{'x' if status == 'COVERED' else ' '}] {esc(path)} lines {start}-{end}: {status}"
              f"{'; turns ' + str(turns_used) if turns_used else ''}"
              f"{'; only partly shown: ' + str(partial_only) if partial_only else ''}"
              f"{'; never shown: ' + str(missing) if missing and got else ''}")
    for literal in args.expect:
        where = [name for name, text in seen if literal in text]
        print(f"  [{'x' if where else ' '}] literal {esc(literal)!r}: "
              f"{'seen in ' + '; '.join(where[:4]) if where else 'never appeared in anything the model saw'}")
    if not spans and not args.expect:
        print('  (pass --span and --expect to check specific lines and literals)')


if __name__ == '__main__':
    main()
