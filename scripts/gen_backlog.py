#!/usr/bin/env python3
"""Build waves from docs/product/slices.yaml.

Usage: python3 scripts/gen_backlog.py             # refresh generated parts of backlog.md + epics Files fields
       python3 scripts/gen_backlog.py --wave N    # also write ./stories.yaml (hermad plan-devs) for wave N
       python3 scripts/gen_backlog.py --check     # exit 1 if backlog.md or epics Files fields are stale

A wave holds slices whose deps are all in earlier waves, whose `files` are pairwise disjoint,
at most MAX_PER_WAVE of them. Within a wave, slices on the longest remaining path go first.
"""
import re, sys
from pathlib import Path
import yaml

ROOT = Path(__file__).resolve().parent.parent
SLICES = ROOT / "docs/product/slices.yaml"
STORIES = ROOT / "docs/product/stories.yaml"
EPICS = ROOT / "docs/product/epics-and-stories.md"
BACKLOG = ROOT / "docs/product/backlog.md"
OUT = ROOT / "stories.yaml"
MAX_PER_WAVE = 4
C = "src-tauri/crates/"
MIG = C + "boba-store/migrations/"

# v1.1 / icebox stories have no slices yet; their exact paths live here.
ICEBOX_FILES = {
    "11.4": [C + "boba-sources/src/session/latam/**"],
    "12.1": [C + "boba-sources/src/infojobs/**", C + "boba-app/src/apply/sanctioned/**"],
    "12.2": [C + "boba-sources/src/html_scrape/**"],
    "12.3": [C + "boba-sources/src/external_process/**"],
    "13.1": [C + "boba-app/src/followups/**", "agent-pack/skills/follow_up/**"],
    "13.2": [C + "boba-app/src/interview_prep/**", "agent-pack/skills/interview_prep/**"],
    "13.3": [C + "boba-domain/src/scoring/calibration/**"],
}


def load():
    slices = yaml.safe_load(SLICES.read_text())
    for s in slices:  # `last: true` = runs after every other slice (final QA pass)
        if s.get("last"):
            s["deps"] = list(dict.fromkeys(s["deps"] + [o["id"] for o in slices if o is not s]))
    stories = {s["id"]: s for s in yaml.safe_load(STORIES.read_text())["stories"]}
    return slices, stories


def norm(p):
    return p[:-3] if p.endswith("/**") else p


def overlap(a, b):
    a, b = norm(a), norm(b)
    return a == b or a.startswith(b + "/") or b.startswith(a + "/")


def raw_files(s):
    out = []
    for f in s["files"]:
        f = f.replace("@c/", C)
        m = re.fullmatch(r"\{loc:(\w+)\}", f)
        if m:
            out += [f"locales/en/{m[1]}.json", f"locales/es/{m[1]}.json"]
        else:
            out.append(f)
    return out


def validate(slices, stories):
    errs, ids = [], [s["id"] for s in slices]
    if len(ids) != len(set(ids)):
        errs.append("duplicate slice ids")
    known = set(ids)
    for s in slices:
        for d in s["deps"]:
            if d not in known:
                errs.append(f"{s['id']}: unknown dep {d}")
        p = stories.get(s["parent"])
        if not p:
            errs.append(f"{s['id']}: unknown parent {s['parent']}")
            continue
        for i in s.get("ac_from", []):
            if i >= len(p["ac"]):
                errs.append(f"{s['id']}: ac_from {i} out of range for {s['parent']}")
        for f in s["files"]:
            if "," in f or "#" in f or "{en" in f:
                errs.append(f"{s['id']}: file entry unsafe for hermad parser: {f}")
    parents = {s["parent"] for s in slices}
    for sid, st in stories.items():
        if st["phase"] == "MVP" and sid not in parents:
            errs.append(f"MVP story {sid} has no slice")
    # split parents: every parent AC must land in some slice
    by_parent = {}
    for s in slices:
        by_parent.setdefault(s["parent"], []).append(s)
    for pid, group in by_parent.items():
        group = [s for s in group if not s.get("followup")]
        if any("ac_from" not in s for s in group):
            continue
        used = {i for s in group for i in s["ac_from"]}
        moved = any(pid in a for s in slices for a in s.get("ac_extra", []) if "moved from story" in a)
        missing = set(range(len(stories[pid]["ac"]))) - used
        if missing and not moved:
            errs.append(f"story {pid}: ACs {sorted(missing)} not assigned to any slice")
    return errs


def ancestors(slices):
    deps = {s["id"]: s["deps"] for s in slices}
    memo = {}

    def anc(n):
        if n not in memo:
            memo[n] = set()
            for d in deps[n]:
                memo[n] |= {d} | anc(d)
        return memo[n]
    return {n: anc(n) for n in deps}


def plan(slices):
    by_id = {s["id"]: s for s in slices}
    files = {s["id"]: [f for f in raw_files(s) if not f.startswith("{mig:")] + [MIG] * any(f.startswith("{mig:") for f in s["files"]) for s in slices}
    # A slice with a migration also touches the shared migrations folder only through its own new file,
    # so the folder itself is not a conflict; drop the placeholder used above.
    files = {k: [f for f in v if f != MIG] for k, v in files.items()}
    order = {s["id"]: i for i, s in enumerate(slices)}
    children = {s["id"]: [] for s in slices}
    for s in slices:
        for d in s["deps"]:
            children[d].append(s["id"])
    depth = {}

    def tail(n):
        if n not in depth:
            depth[n] = 1 + max((tail(c) for c in children[n]), default=0)
        return depth[n]
    waves, placed = [], {}
    while len(placed) < len(slices):
        ready = [s["id"] for s in slices if s["id"] not in placed and all(d in placed for d in s["deps"])]
        if not ready:
            sys.exit("dependency cycle among: " + ", ".join(s["id"] for s in slices if s["id"] not in placed))
        ready.sort(key=lambda n: (not by_id[n].get("first"), -tail(n), order[n]))  # `first: true` = dispatch as soon as ready
        # `wave: N` pins an already-dispatched slice to its historical wave (overlaps there are reported, not re-planned)
        wave = [n for n in ready if by_id[n].get("wave") == len(waves) + 1]
        ready = [n for n in ready if not by_id[n].get("wave")]
        for n in ready:
            if len(wave) >= MAX_PER_WAVE:
                break
            if all(not overlap(a, b) for m in wave for a in files[n] for b in files[m]):
                wave.append(n)
        # the scaffold always lands alone
        if "S1.1" in wave:
            wave = ["S1.1"]
        for n in wave:
            placed[n] = len(waves) + 1
        waves.append(wave)
    mig_no, k = {}, 1
    for w in waves:
        for n in w:
            for f in by_id[n]["files"]:
                m = re.fullmatch(r"\{mig:(\w+)\}", f)
                if m:
                    mig_no[(n, m[1])] = f"{MIG}{k:04d}_{m[1]}.sql"
                    k += 1
    return waves, placed, files, mig_no, tail


def expanded(s, mig_no):
    out = []
    for f in raw_files(s):
        m = re.fullmatch(r"\{mig:(\w+)\}", f)
        out.append(mig_no[(s["id"], m[1])] if m else f)
    return out


def ac_text(a):
    a = re.sub(r"^Given (.*?) When (.*?) Then ", lambda m: f"Given {m[1]}, when {m[2]}, then ", a, flags=re.S)
    if a.startswith("And "):
        a = "Given this slice is merged, when the reviewer checks it, then " + a[4:5].lower() + a[5:]
    a = a.replace('"', "'").replace("#", "no.")
    return a.replace('", "', "', '")


def slice_acs(s, stories):
    parent = stories[s["parent"]]["ac"]
    idx = s.get("ac_from", range(len(parent)))
    return [ac_text(parent[i]) for i in idx] + [ac_text(a) for a in s.get("ac_extra", [])]


def covers(s, stories):
    return list(dict.fromkeys(stories[s["parent"]]["covers"] + s.get("covers_extra", [])))


def q(v):
    return '"' + str(v).replace('"', "'") + '"'


def render_wave_yaml(n, waves, slices, stories, mig_no, placed):
    by_id = {s["id"]: s for s in slices}
    lines = [f"# Wave {n} of {len(waves)} — generated by scripts/gen_backlog.py --wave {n} from docs/product/slices.yaml. Do not edit.",
             "# Earlier waves are merged before this file is dispatched, so depends_on lists only ids inside this wave."]
    for sid in waves[n - 1]:
        s = by_id[sid]
        inside = [d for d in s["deps"] if placed[d] == n]
        lines += [f"- id: {sid}",
                  "  initiative: MVP",
                  f"  epic: E{s['parent'].split('.')[0]}",
                  f"  title: {q(s['title'])}",
                  f"  parent_story: {q(s['parent'])}",
                  f"  depends_on: [{', '.join(inside)}]",
                  f"  after: [{'all-other-slices' if s.get('last') else ', '.join(s['deps'])}]",
                  f"  files: [{', '.join(expanded(s, mig_no))}]",
                  f"  covers: [{', '.join(covers(s, stories))}]",
                  f"  ac: [{', '.join(q(a) for a in slice_acs(s, stories))}]"]
        if s.get("gate"):
            lines.append(f"  gate: {s['gate']}")
        if s.get("notes"):
            lines.append(f"  notes: {q(s['notes'])}")
    return "\n".join(lines) + "\n"


def render_backlog_sections(waves, slices, stories, mig_no, tail):
    by_id = {s["id"]: s for s in slices}
    size = lambda s: len(slice_acs(s, stories)) + len(raw_files(s)) / 2
    est = lambda s: "L" if size(s) >= 7 else "M" if size(s) >= 4 else "S"
    out = ["| Wave | Slice | Story | Title | After | Est | Gate |", "|---|---|---|---|---|---|---|"]
    for i, w in enumerate(waves, 1):
        for sid in w:
            s = by_id[sid]
            out.append(f"| {i} | {sid} | {s['parent']} | {s['title']} | {'all other slices' if s.get('last') else ', '.join(s['deps']) or '—'} | {est(s)} | {'⚠ Diego' if s.get('gate') else ''} |")
    summary = [f"**{len(slices)} slices from {len({s['parent'] for s in slices})} MVP stories, in {len(waves)} waves** "
               f"(max {MAX_PER_WAVE} parallel devs). Critical path: {max(tail(s['id']) for s in slices)} slices.", "",
               "| Wave | Slices | Parallel devs |", "|---|---|---|"]
    summary += [f"| {i} | {', '.join(w)} | {len(w)} |" for i, w in enumerate(waves, 1)]
    files_sec = []
    for i, w in enumerate(waves, 1):
        for sid in w:
            files_sec.append(f"- **{sid}** (wave {i}): " + ", ".join(f"`{f}`" for f in expanded(by_id[sid], mig_no)))
    scaffold = raw_files(next(s for s in slices if s["id"] == "S1.1"))
    owned = sorted({f for s in slices if s["id"] != "S1.1" for f in expanded(s, mig_no)
                    if any(overlap(f, g) for g in scaffold) and not f.startswith(MIG)})
    later = {}
    for s in slices:
        for f in expanded(s, mig_no):
            if s["id"] != "S1.1" and not any(overlap(f, g) for g in scaffold):
                later.setdefault(s["id"], []).append(f)
    groups = {}
    for f in owned:
        key = "/".join(f.split("/")[:3]) if f.startswith(C) else "/".join(f.split("/")[:2]) if "/" in f else f
        groups.setdefault(key, []).append(f)
    modmap = [f"- `{k}`: " + ", ".join(f"`{f[len(k) + 1:] if f.startswith(k + '/') else f}`" for f in v) for k, v in sorted(groups.items())]
    modmap += ["", "**Not stubbed by S1.1** (outside its files; each is created by its owning slice):", ""]
    modmap += [f"- **{sid}**: " + ", ".join(f"`{f}`" for f in fs) for sid, fs in later.items()]
    modmap += ["- **every slice with a migration**: its own `" + MIG + "NNNN_name.sql` (see Slice files)"]
    return {"summary": "\n".join(summary), "waves": "\n".join(out), "files": "\n".join(files_sec), "modmap": "\n".join(modmap)}


def splice(text, name, body):
    pat = re.compile(rf"(<!-- gen:{name}:begin -->\n).*?(<!-- gen:{name}:end -->)", re.S)
    if not pat.search(text):
        sys.exit(f"backlog.md is missing the gen:{name} markers")
    return pat.sub(lambda m: m[1] + body + "\n" + m[2], text)


def sync_epics(text, slices, mig_no):
    files_by_parent = {}
    for s in slices:
        files_by_parent.setdefault(s["parent"], []).extend(expanded(s, mig_no))
    files_by_parent.update(ICEBOX_FILES)

    def fix(part):
        head, _, body = part.partition("\n")
        sid = head.split(":", 1)[0].strip()
        if sid not in files_by_parent:
            return part
        fl = ", ".join(f"`{f}`" for f in dict.fromkeys(files_by_parent[sid]))
        line = body.split("\n", 1)[0]
        if "**Files:**" in line:
            new = re.sub(r"\*\*Files:\*\* .*? · \*\*Covers:\*\*", lambda m: f"**Files:** {fl} · **Covers:**", line)
        else:
            new = line.replace(" · **Covers:**", f" · **Files:** {fl} · **Covers:**", 1)
        return head + "\n" + body.replace(line, new, 1)
    parts = re.split(r"(?m)^### Story ", text)
    return parts[0] + "".join("### Story " + fix(p) for p in parts[1:])


def main():
    slices, stories = load()
    errs = validate(slices, stories)
    if errs:
        sys.exit("\n".join(errs))
    waves, placed, files, mig_no, tail = plan(slices)
    anc = ancestors(slices)
    cross = [(a["id"], b["id"]) for i, a in enumerate(slices) for b in slices[i + 1:]
             if a["id"] not in anc[b["id"]] and b["id"] not in anc[a["id"]] and placed[a["id"]] != placed[b["id"]]
             and any(overlap(x, y) for x in files[a["id"]] for y in files[b["id"]])]
    pinned = [(a, b, x) for w in waves for i, a in enumerate(w) for b in w[i + 1:]
              for x in files[a] for y in files[b] if overlap(x, y)]
    for a, b, x in pinned:
        print(f"warning: pinned wave shares {x} between {a} and {b} (merge one, rebase the other)")
    secs = render_backlog_sections(waves, slices, stories, mig_no, tail)
    bl = BACKLOG.read_text()
    for k, v in secs.items():
        bl = splice(bl, k, v)
    ep = sync_epics(EPICS.read_text(), slices, mig_no)
    if "--check" in sys.argv:
        stale = [p.name for p, t in ((BACKLOG, bl), (EPICS, ep)) if p.read_text() != t]
        if stale:
            sys.exit("stale: " + ", ".join(stale) + " (run scripts/gen_backlog.py, then scripts/gen_stories.py)")
        print(f"ok: {len(slices)} slices, {len(waves)} waves")
        return
    BACKLOG.write_text(bl)
    EPICS.write_text(ep)
    if "--wave" in sys.argv:
        n = int(sys.argv[sys.argv.index("--wave") + 1])
        OUT.write_text(render_wave_yaml(n, waves, slices, stories, mig_no, placed))
        print(f"wave {n} -> {OUT.relative_to(ROOT)}: {', '.join(waves[n - 1])}")
    print(f"{len(slices)} slices, {len(waves)} waves; cross-wave file sharing between non-ancestor slices: {len(cross)} pair(s) (safe while waves run one at a time)")


if __name__ == "__main__":
    main()
