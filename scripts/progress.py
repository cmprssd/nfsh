"""The project's numbers, counted from git, the tree and the recorded inventory, so the
docs never go stale.

    python3 scripts/progress.py            rewrites the generated blocks of docs/OVERVIEW.md
    python3 scripts/progress.py --check    fails if they differ from what it would write
    python3 scripts/progress.py --json F   writes every number to F (the progress page's data)

A generated block sits between `<!-- progress:NAME -->` and `<!-- /progress -->`;
text outside the blocks is written by hand. The history is main's (up to where this
branch leaves it) plus this change, if it is not on main yet, so a PR's numbers
already count it and stay right once it merges. The install's numbers come from
docs/INVENTORY.md, a real run of `nfsh inventory`, updated by the PR that reruns it.
Standard library only: .local-ci runs the check.
"""
import collections
import json
import pathlib
import re
import subprocess
import sys
import textwrap

ROOT = pathlib.Path(__file__).resolve().parent.parent
OVERVIEW = ROOT / "docs/OVERVIEW.md"
INVENTORY = ROOT / "docs/INVENTORY.md"
# Lines added and removed leave out the overview itself, whose numbers would otherwise
# count their own rewriting.
NOT_COUNTED = ":!docs/OVERVIEW.md"


def git(*args):
    return subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True, check=True).stdout


def history():
    """Main's changes (its first-parent line: one per squashed or merged PR) up to this
    branch's merge base, oldest first, plus this change if it is not on main yet."""
    base = git("merge-base", "HEAD", "origin/main").strip()
    on_main = subprocess.run(["git", "-C", str(ROOT), "merge-base", "--is-ancestor", "HEAD",
                              "origin/main"]).returncode == 0
    changes = [change(*line.split("\t", 2), "") for line in git(
        "log", "--reverse", "--first-parent", "--format=%h%x09%ad%x09%s", "--date=short", base).splitlines()]
    # Lines changed come from a second log limited to the counted paths: limiting the
    # first would drop the changes that touch only the overview.
    stat = git("log", "--first-parent", "--diff-merges=first-parent", "--format=%x09%h", "--shortstat",
               base, "--", ".", NOT_COUNTED)
    counted = {}
    for line in stat.splitlines():
        if line.startswith("\t"):
            h = line[1:]
        elif line.strip():
            counted[h] = change("", "", "", line)
    for c in changes:
        c.update(counted.get(c["hash"], {}))
    if not on_main:
        # This change, counted as the squash merge it becomes.
        date = git("log", "-1", "--format=%ad", "--date=short", "HEAD").strip()
        # Its type is the branch's first commit's, which the PR is titled after: a later
        # commit (regenerated numbers, a fix-up) does not change what the PR is.
        subject = git("log", "--reverse", "--format=%s", f"{base}..HEAD").splitlines()[0]
        diff = git("diff", "--shortstat", base, "HEAD", "--", ".", NOT_COUNTED)
        changes.append({**change("this", date, subject, diff), "pr": "this"})
    return changes, int(git("rev-list", "--count", base)) + (0 if on_main else 1)


def change(h, date, subject, shortstat):
    """One change on main; a merge commit's subject is read for its PR and branch type."""
    merge = re.match(r"Merge pull request #(\d+) from [^/]+/(\w+)/", subject)
    squash = re.search(r"\(#(\d+)\)$", subject)
    c = {"added": int((re.search(r"(\d+) insertion", shortstat) or [0, 0])[1]),
         "removed": int((re.search(r"(\d+) deletion", shortstat) or [0, 0])[1])}
    if h:
        c.update(hash=h, date=date, subject=subject,
                 pr=(merge or squash)[1] if merge or squash else None,
                 type=merge[2] if merge else re.match(r"(\w+)", subject)[1])
    return c


def code():
    """Rust lines (sources and tests) and #[test]s per crate."""
    files = git("ls-files", "-z", "--", "crates").split("\0")
    crates = collections.OrderedDict()
    for f in sorted(files):
        path = ROOT / f
        if not f.endswith(".rs") or not path.is_file():
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        c = crates.setdefault(f.split("/")[1], {"lines": 0, "tests": 0})
        c["lines"] += text.count("\n")
        c["tests"] += len(re.findall(r"#\[test\]", text))
    return crates


def decisions():
    heads = [l for l in (ROOT / "docs/DECISIONS.md").read_text().splitlines() if l.startswith("## ")]
    return {
        "entries": len(heads),
        "owner": sum("chosen by the owner" in h for h in heads),
        "claude": sum("decided by Claude" in h for h in heads),
    }


def gib(s):
    """A size as the inventory prints it (`12.41 GiB`, `821.92 MiB`), in GiB."""
    n, unit = s.split()
    return float(n) / (1024 if unit == "MiB" else 1)


def inventory():
    """The install's headline numbers from the recorded `nfsh inventory` run. Fails loudly
    if the command's wording changed, so the overview cannot quietly lose them."""
    text = INVENTORY.read_text()

    def find(pattern):
        m = re.search(pattern, text)
        if not m:
            sys.exit(f"docs/INVENTORY.md no longer matches {pattern!r}: update scripts/progress.py")
        return m

    measured = find(r"`nfsh inventory` on (\d{4}-\d\d-\d\d)")[1]
    disk = find(r"(\d+) files, ([\d.]+ GiB) \((\d+) bytes\)")
    live = find(r"Of ([\d.]+ GiB) in cas archives, current entries \(patch applied\) use ([\d.]+ GiB) "
                r"\(([\d.]+)%\); the other ([\d.]+ GiB)")
    sb = find(r"(\d+) superbundles hold (\d+) bundles listing (\d+) ebx, (\d+) res and (\d+) chunk entries; "
              r"distinct by name or id: (\d+) ebx, (\d+) res, (\d+) chunks")
    on_disk = text.split("## On disk", 1)[1].split("\n## ", 1)[0]
    kinds = [{"what": m[1], "files": int(m[2]), "gib": gib(m[3])}
             for m in re.finditer(r"^\| ([^|`]+?) \| (\d+) \| ([\d.]+ [GM]iB) \|", on_disk, re.M)]
    sbs = [{"name": m[1], "gib": gib(m[2])}
           for m in re.finditer(r"^\| `(Win32/[^`]+)` \|(?: \d+ \|){4} ([\d.]+ [GM]iB) \|$", text, re.M)]
    return {
        "measured": measured,
        "files": int(disk[1]), "gib": gib(disk[2]), "bytes": int(disk[3]),
        "cas_gib": gib(live[1]), "live_gib": gib(live[2]), "live_pct": float(live[3]), "dead_gib": gib(live[4]),
        "superbundles": int(sb[1]), "bundles": int(sb[2]),
        "refs": {"ebx": int(sb[3]), "res": int(sb[4]), "chunks": int(sb[5])},
        "distinct": {"ebx": int(sb[6]), "res": int(sb[7]), "chunks": int(sb[8])},
        "kinds": kinds,
        "superbundle_sizes": sbs,
    }


def stats():
    commits, ncommits = history()
    prs = [c for c in commits if c["pr"]]
    types = collections.Counter(c["type"] for c in prs)
    days = collections.Counter(c["date"] for c in prs)
    crates = code()
    return {
        "as_of": commits[-1]["date"],
        "last_pr": next((c["pr"] for c in reversed(prs) if c["pr"] != "this"), None),
        "commits": ncommits,
        "prs": len(prs),
        "first_day": commits[0]["date"],
        "pr_types": dict(types.most_common()),
        "prs_per_day": dict(sorted(days.items())),
        "added": sum(c["added"] for c in commits),
        "removed": sum(c["removed"] for c in commits),
        "tracked_files": len([f for f in git("ls-files", "-z").split("\0") if f]),
        "crates": crates,
        "rust_lines": sum(c["lines"] for c in crates.values()),
        "tests": sum(c["tests"] for c in crates.values()),
        "decisions": decisions(),
        "install": inventory(),
        "history": [{k: c[k] for k in ("hash", "date", "subject", "pr", "type", "added", "removed")} for c in prs],
    }


def growth(changes):
    """Lines of Rust and #[test]s in the tree after each change on main (the page's chart)."""
    cache = {}
    cat = subprocess.Popen(["git", "-C", str(ROOT), "cat-file", "--batch"], stdin=subprocess.PIPE,
                           stdout=subprocess.PIPE)

    def blob(oid):
        if oid not in cache:
            cat.stdin.write(oid.encode() + b"\n")
            cat.stdin.flush()
            size = int(cat.stdout.readline().split()[2])
            text = cat.stdout.read(size + 1)[:-1]
            cache[oid] = (text.count(b"\n"), text.count(b"#[test]"))
        return cache[oid]

    out = []
    for c in changes:
        # Only merged PRs: this change has no tree on main yet, and the first commit
        # (the brief) is not a PR.
        if c["hash"] == "this" or not c["pr"]:
            continue
        lines = tests = 0
        for row in git("ls-tree", "-r", c["hash"], "--", "crates").splitlines():
            meta, path = row.split("\t", 1)
            if path.endswith(".rs"):
                n, t = blob(meta.split()[2])
                lines, tests = lines + n, tests + t
        out.append({"date": c["date"], "pr": c["pr"], "rust_lines": lines, "tests": tests})
    cat.stdin.close()
    return out


def day(d):
    months = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split()
    y, m, dd = d.split("-")
    return f"{months[int(m) - 1]} {int(dd)}"


def blocks(s):
    """The text of each generated block, by name."""
    b = {}
    # Counted, not named by number: a branch counts its own PR before it has one.
    b["asof"] = (f"Numbers in the generated parts below are as of {s['as_of']} "
                 f"({s['prs']} merged PRs), counted by `scripts/progress.py`.")
    b["code"] = (f"{len(s['crates'])} crates in one Cargo workspace, {s['rust_lines']:,} lines of Rust "
                 f"(sources and tests), {s['tests']} tests.")
    b["process"] = (f"So far: {s['decisions']['entries']} decision entries, {s['decisions']['owner']} "
                    f"chosen by the owner, {s['decisions']['claude']} made by Claude for review.")
    types = ", ".join(f"{n} {t}" for t, n in s["pr_types"].items())
    span = len(s["prs_per_day"])
    labels = ", ".join(f'"{day(d)}"' for d in s["prs_per_day"])
    counts = ", ".join(str(n) for n in s["prs_per_day"].values())
    top = max(s["prs_per_day"].values())
    b["history"] = "\n".join([
        f"{s['commits']} commits, {s['prs']} merged PRs from {s['first_day']} to {s['as_of']}, "
        f"on {span} working days: {types}. About {s['added']:,} lines added and "
        f"{s['removed']:,} removed; {s['tracked_files']} tracked files.",
        "",
        "```mermaid",
        "xychart-beta",
        '    title "PRs merged per working day"',
        f"    x-axis [{labels}]",
        f'    y-axis "PRs" 0 --> {top + 2}',
        f"    bar [{counts}]",
        "```",
    ])
    i = s["install"]
    b["install"] = "\n".join([
        f"Measured by `nfsh inventory` on {i['measured']} ([INVENTORY.md](INVENTORY.md)):",
        "",
        "| What | Count | Size |",
        "|------|------:|------:|",
        f"| Files in the install | {i['files']:,} | {i['gib']:.2f} GiB |",
        f"| cas archives (stored asset bytes) | | {i['cas_gib']:.2f} GiB |",
        f"| ... still read after the patch | | {i['live_gib']:.2f} GiB ({i['live_pct']:.1f}%) |",
        f"| ... replaced by the patch, dead | | {i['dead_gib']:.2f} GiB |",
        f"| Superbundles, bundles | {i['superbundles']}, {i['bundles']:,} | |",
        f"| Ebx, res, chunk references | {i['refs']['ebx']:,}, {i['refs']['res']:,}, "
        f"{i['refs']['chunks']:,} | |",
        f"| ... distinct | {i['distinct']['ebx']:,}, {i['distinct']['res']:,}, "
        f"{i['distinct']['chunks']:,} | |",
    ])
    b["crates"] = list(s["crates"].items())
    return b


def wrap(text, width):
    """Wraps prose lines as the hand-written text is; tables and charts stay as they are."""
    fenced = False
    out = []
    for line in text.splitlines():
        fenced ^= line.startswith("```")
        prose = not (fenced or line.startswith(("|", "```")))
        out.append(textwrap.fill(line, width) if prose and line else line)
    return "\n".join(out)


def apply(text, b):
    def table(old, rows):
        # Rewrites the line and test counts of the crate table; the prose columns stay.
        out = []
        for line in old.splitlines():
            m = re.match(r"\| `([\w-]+)` \|[^|]*\|[^|]*\|(.*)$", line)
            if m and m[1] in dict(rows):
                c = dict(rows)[m[1]]
                line = f"| `{m[1]}` | {c['lines']:,} | {c['tests']} |{m[2]}"
            out.append(line)
        names = {re.match(r"\| `([\w-]+)`", l)[1] for l in out if re.match(r"\| `([\w-]+)`", l)}
        for name, c in rows:
            if name not in names:
                out.append(f"| `{name}` | {c['lines']:,} | {c['tests']} | (describe) |")
        return "\n".join(out)

    def sub(m):
        # A block inside a list item keeps the item's indentation.
        indent, name, body = m[1], m[2], m[3]
        if name == "crates":
            new = table(body.strip("\n"), b["crates"])
        else:
            new = b.get(name, body.strip("\n"))
        new = "\n".join(indent + l if l else l for l in wrap(new, 88 - len(indent)).splitlines())
        return f"{indent}<!-- progress:{name} -->\n{new}\n{indent}<!-- /progress -->"

    return re.sub(r"^([ \t]*)<!-- progress:(\w+) -->\n(.*?)^[ \t]*<!-- /progress -->", sub, text,
                  flags=re.S | re.M)


def main():
    s = stats()
    if "--json" in sys.argv:
        s["growth"] = growth(history()[0])
        pathlib.Path(sys.argv[sys.argv.index("--json") + 1]).write_text(json.dumps(s, indent=1))
        return
    old = OVERVIEW.read_text()
    new = apply(old, blocks(s))
    if "--check" in sys.argv:
        if new != old:
            sys.exit("docs/OVERVIEW.md's numbers are stale: run python3 scripts/progress.py")
        return
    OVERVIEW.write_text(new)


main()
