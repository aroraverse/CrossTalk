"""Turn the showcase run logs into a timeline of what each agent printed.

python -I extract.py <logs_dir> <out.json>
Each event: {"t": unix_ms, "agent": "claude|codex|kiro|opencode", "kind": "tool|text|cmd|done", "text": "..."}
Only real lines from the logs; progress noise is dropped and long lines are cut.
"""
import json
import re
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m|\[0m")


def fix(s):
    """Undo UTF-8 that a Windows console decoded as cp437."""
    try:
        return s.encode("cp437").decode("utf-8")
    except (UnicodeEncodeError, UnicodeDecodeError):
        return s


def short_key(v):
    return str(v).split("-refund-service")[0]


def tool_line(name, args):
    name = name.replace("mcp__crosstalk__", "").replace("crosstalk_", "").replace("@crosstalk/", "")
    if name == "send_message":
        to = short_key(args.get("to", "?"))
        msg = (args.get("message") or "").replace("\n", " ")
        rt = "  (reply)" if args.get("reply_to") else ""
        return f"crosstalk.send_message → {to}{rt}: {msg}"
    if name == "resolve_recipient":
        return f"crosstalk.resolve_recipient(\"{args.get('selector', '')}\")"
    if name == "wait_for_reply":
        return f"crosstalk.wait_for_reply({args.get('message_id', '')}) … waiting"
    if name == "ask_user":
        return f"crosstalk.ask_user: {args.get('question', '')}"
    if name in ("check_inbox", "list_team"):
        return f"crosstalk.{name}()"
    return None


def events(path, agent):
    out = []
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if "\t" not in raw:
            continue
        ts, line = raw.split("\t", 1)
        try:
            t = int(ts)
        except ValueError:
            continue
        line = ANSI.sub("", fix(line)).rstrip()
        if not line or line.startswith(("START ", "END ")):
            continue
        add = lambda kind, text: out.append({"t": t, "agent": agent, "kind": kind, "text": text[:400]})
        if agent in ("claude", "codex") and line.startswith("{"):
            try:
                ev = json.loads(line)
            except json.JSONDecodeError:
                continue
            if agent == "claude":
                if ev.get("type") == "assistant":
                    for c in ev["message"].get("content", []):
                        if c.get("type") == "text" and c["text"].strip():
                            add("text", c["text"].strip())
                        elif c.get("type") == "tool_use":
                            n, a = c["name"], c.get("input", {})
                            tl = tool_line(n, a)
                            if tl:
                                add("tool", tl)
                            elif n in ("Read", "Edit", "Write"):
                                add("cmd", f"{n}({Path(a.get('file_path', '')).name})")
                            elif n == "Bash":
                                add("cmd", f"$ {a.get('command', '')}")
                elif ev.get("type") == "user":
                    for c in ev["message"].get("content", []) if isinstance(ev["message"].get("content"), list) else []:
                        if c.get("type") == "tool_result":
                            body = c.get("content")
                            text = body if isinstance(body, str) else " ".join(x.get("text", "") for x in body or [] if isinstance(x, dict))
                            m = re.search(r"Ran \d+ tests? in [\d.]+s\s+OK", text)
                            if m:
                                add("done", " ".join(m.group(0).split()))
            else:
                it = ev.get("item") or {}
                if ev.get("type") != "item.completed":
                    continue
                if it.get("type") == "agent_message":
                    add("text", it["text"].strip())
                elif it.get("type") == "mcp_tool_call":
                    tl = tool_line(it.get("tool", ""), it.get("arguments") or {})
                    if tl:
                        add("tool", tl)
                elif it.get("type") == "command_execution":
                    add("cmd", f"$ {it.get('command', '')}")
            continue
        if agent == "kiro":
            if line.startswith("[INFO]") or line.startswith("[ERROR]") or "status:" in line or line.startswith("(Use"):
                continue
            m = re.match(r"\[tool\] (.*)", line)
            if m:
                name = m.group(1)
                if name.startswith("@crosstalk/"):
                    add("tool", f"crosstalk.{name.split('/', 1)[1]}()")
                elif name not in ("Tool Load",):
                    add("cmd", name)
                continue
            add("done" if re.search(r"Ran \d+ tests", line) else "text", line)
            continue
        if agent == "opencode":
            if line.startswith(">") and "Sisyphus" in line:
                continue
            m = re.match(r"[⚙⚡]\s*(\S+)\s*(.*)", line)
            if m:
                name, rest = m.group(1), m.group(2)
                try:
                    a = json.loads(rest) if rest.startswith("{") else {}
                except json.JSONDecodeError:
                    a = {}
                tl = tool_line(name, a)
                add("tool", tl or f"{name}()")
                continue
            m = re.match(r"→\s*(.*)", line)
            if m:
                add("cmd", m.group(1))
                continue
            add("text", line)
    return out


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    logs, dest = Path(sys.argv[1]), Path(sys.argv[2])
    allev = []
    for f in sorted(logs.glob("*.log")):
        agent = f.name.split("-", 1)[0]
        allev += events(f, agent)
    allev.sort(key=lambda e: e["t"])
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(json.dumps(allev, ensure_ascii=False, indent=1), encoding="utf-8")
    print(len(allev), "events")
    for e in allev:
        print(e["t"], e["agent"], e["kind"], "|", e["text"][:120])


if __name__ == "__main__":
    main()
