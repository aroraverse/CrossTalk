"""Live, read-only view of the Crosstalk team for the e2e-team folder.

Usage:  python -I monitor.py            (refresh every 2 s, Ctrl+C to stop)
        python -I monitor.py --report   (print the end-to-end report once)
"""
import os
import sqlite3
import sys
import time

DB = os.path.join(os.environ["APPDATA"], "crosstalk", "crosstalk.db")
FOLDER = "e2e-team"


def connect():
    return sqlite3.connect(f"file:{DB}?mode=ro", uri=True, timeout=2)


def sessions(c):
    now = int(time.time())
    rows = c.execute(
        "select session_key, agent_type, status, last_seen, working_dir from sessions "
        "where working_dir like ? order by session_key", (f"%{FOLDER}",)).fetchall()
    return [(k, t, s, now - seen) for k, t, s, seen, _ in rows]


def messages(c, keys, limit=14):
    if not keys:
        return []
    marks = ",".join("?" * len(keys))
    return c.execute(
        f"select id, sender_key, recipient_key, body, state, reply_to, created_at from messages "
        f"where sender_key in ({marks}) or recipient_key in ({marks}) "
        f"order by created_at desc, id desc limit ?", (*keys, *keys, limit)).fetchall()[::-1]


def short(key):
    return key.split("-e2e-team-")[0] if "-e2e-team-" in key else key


def draw():
    c = connect()
    rows = sessions(c)
    keys = [r[0] for r in rows]
    out = [f"CROSSTALK LIVE MONITOR   {time.strftime('%H:%M:%S')}   folder: {FOLDER}", ""]
    out.append(f"{'AGENT':10} {'SESSION KEY':36} {'TYPE':12} {'STATUS':8} SEEN")
    for k, t, s, age in rows:
        out.append(f"{short(k):10} {k:36} {t or '':12} {s:8} {age}s ago")
    if not rows:
        out.append("(no e2e-team sessions yet: run step 1)")
    out += ["", f"{'TIME':8} {'FROM':9} -> {'TO':9} {'STATE':6} {'REPLY TO':22} BODY"]
    for mid, frm, to, body, state, reply_to, at in messages(c, keys):
        line = body.replace("\n", " ")
        out.append(f"{time.strftime('%H:%M:%S', time.localtime(at))} {short(frm):9} -> {short(to):9} "
                   f"{state:6} {(reply_to or '-')[:22]:22} {line[:70]}")
    c.close()
    return "\n".join(out)


def report():
    c = connect()
    keys = [r[0] for r in sessions(c)]
    rows = messages(c, keys, limit=200)
    by_id = {r[0]: r for r in rows}
    print(f"Sessions in {FOLDER}: {', '.join(short(k) for k in keys)}  ({len(keys)})")
    print()
    for mid, frm, to, body, state, reply_to, at in rows:
        if body.startswith(("E2E-PING", "E2E-RELAY")):
            replies = [r for r in rows if r[5] == mid]
            ok = replies and replies[0][1] == to and replies[0][2] == frm
            verdict = "OK  " if ok else "MISS"
            answer = replies[0][3] if replies else "(no correlated reply)"
            print(f"[{verdict}] {short(frm)} -> {short(to)}: {body[:44]}...")
            print(f"        reply from {short(replies[0][1]) if replies else '-'}: {answer}  (reply_to = {mid})")
    c.close()


if __name__ == "__main__":
    if "--report" in sys.argv:
        report()
        sys.exit(0)
    try:
        while True:
            screen = draw()
            os.system("cls" if os.name == "nt" else "clear")
            print(screen, flush=True)
            time.sleep(2)
    except KeyboardInterrupt:
        pass
