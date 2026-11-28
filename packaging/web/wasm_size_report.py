#!/usr/bin/env python3
"""Print where a wasm module's bytes go: sections, data, and code by crate."""
import re, sys
from collections import defaultdict

def leb(b, i):
    r = s = 0
    while True:
        x = b[i]; i += 1
        r |= (x & 0x7f) << s; s += 7
        if x < 0x80: return r, i

def crate_of(name):
    if name.startswith("_ZN"):
        m = re.match(r"_ZN(\d+)", name)
        if m:
            n = int(m.group(1)); s = name[3 + len(m.group(1)):3 + len(m.group(1)) + n]
            if s.startswith("_$LT$"):
                inner = re.match(r"_\$LT\$(?:\$RF\$|mut\$u20\$)*([A-Za-z_][A-Za-z0-9_]*)", s)
                return (inner.group(1) if inner else s) + " (impl)"
            return s
    if name.startswith("_R"):
        m = re.search(r"C(?:s[0-9A-Za-z]*_)?(\d+)_?([A-Za-z_][A-Za-z0-9_]*)", name)
        if m: return m.group(2)[:int(m.group(1))]
    return "<other: " + name.split("::")[0][:40] + ">" if "::" in name else "<unmangled>"

def main(path, top=25):
    b = open(path, "rb").read()
    assert b[:4] == b"\0asm"
    i = 8; secs = []; names = {}; code_sizes = []; nimports = 0; data_total = 0
    while i < len(b):
        sid = b[i]; i += 1
        size, i = leb(b, i); body = b[i:i + size]
        label = {0:"custom",1:"type",2:"import",3:"function",4:"table",5:"memory",6:"global",7:"export",8:"start",9:"element",10:"code",11:"data",12:"datacount"}.get(sid, str(sid))
        if sid == 0:
            nl, j = leb(body, 0); cname = body[j:j+nl].decode(); label = "custom:" + cname
            if cname == "name":
                j += nl
                while j < len(body):
                    sub = body[j]; j += 1; sl, j = leb(body, j); end = j + sl
                    if sub == 1:
                        cnt, k = leb(body, j)
                        for _ in range(cnt):
                            idx, k = leb(body, k); ln, k = leb(body, k); names[idx] = body[k:k+ln].decode("utf-8", "replace"); k += ln
                    j = end
        elif sid == 2:
            cnt, j = leb(body, 0)
            for _ in range(cnt):
                l, j = leb(body, j); j += l; l, j = leb(body, j); j += l
                kind = body[j]; j += 1
                if kind == 0: nimports += 1; _, j = leb(body, j)
                elif kind == 1: j += 1; fl, j = leb(body, j); _, j = leb(body, j); j = leb(body, j)[1] if fl & 1 else j
                elif kind == 2: fl, j = leb(body, j); _, j = leb(body, j); j = leb(body, j)[1] if fl & 1 else j
                elif kind == 3: j += 2
        elif sid == 10:
            cnt, j = leb(body, 0)
            for _ in range(cnt):
                fs, j2 = leb(body, j); code_sizes.append(fs + (j2 - j)); j = j2 + fs
        elif sid == 11:
            data_total = size
        secs.append((label, size)); i += size
    total = len(b)
    print(f"{path}: {total} bytes")
    for label, size in sorted(secs, key=lambda s: -s[1])[:8]:
        print(f"  {label:<24}{size:>12} bytes  {100*size/total:5.1f}%")
    by_crate = defaultdict(int); funcs = []
    for k, sz in enumerate(code_sizes):
        n = names.get(nimports + k, f"func[{nimports+k}]")
        by_crate[crate_of(n)] += sz; funcs.append((sz, n))
    if not names:
        print("  (no name section: pass the wasm cargo wrote, before wasm-opt, to attribute code to crates)")
        return
    print(f"  largest crates by code size (of {sum(code_sizes)} code bytes):")
    for c, sz in sorted(by_crate.items(), key=lambda x: -x[1])[:top]:
        print(f"    {sz:>10}  {c}")
    print("  largest functions:")
    for sz, n in sorted(funcs, reverse=True)[:10]:
        print(f"    {sz:>10}  {n[:140]}")

if __name__ == "__main__":
    main(sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 25)
