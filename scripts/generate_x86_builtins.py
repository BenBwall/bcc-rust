"""Derive x86 builtin signatures and immediates from shipped header calls.

Clang recognizes calls even with missing arguments. Its AST supplies canonical
signatures; independent valid probes check every signature on all bcc targets.
No Clang BuiltinsX86.td installation or compiler heap allocation is required.
"""
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CLANG = ROOT / "target/llvm/bin/clang.exe"
OUT = ROOT / "target/vector-builtins"
TRIPLES = ["x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl", "x86_64-w64-windows-gnu", "x86_64-pc-windows-msvc"]
SCALARS = {"void":"v", "char":"c", "signed char":"a", "unsigned char":"A", "short":"s", "unsigned short":"S", "int":"i", "unsigned int":"I", "long":"l", "unsigned long":"L", "long long":"q", "unsigned long long":"Q", "float":"f", "double":"d"}
SIZES = {"c":1,"a":1,"A":1,"s":2,"S":2,"i":4,"I":4,"q":8,"Q":8,"f":4,"d":8}

def run(path, *flags):
    return subprocess.run([str(CLANG), "-std=c11", "-fsyntax-only", "-ferror-limit=0", *flags, str(path)], capture_output=True, text=True)

def json_nodes(text):
    decoder = json.JSONDecoder()
    index = 0
    while index < len(text):
        while index < len(text) and text[index].isspace(): index += 1
        if index == len(text): break
        node, index = decoder.raw_decode(text,index)
        yield node

def encode(text):
    if text.endswith(" *"): return "P" + encode(text[:-2])
    if text.startswith("const "): return "C" + encode(text[6:])
    if text.endswith(" const"): return "C" + encode(text[:-6])
    if text.startswith("__attribute__"):
        m = re.search(r"__vector_size__\((\d+) \* sizeof\((.*?)\)\)",text)
        assert m, text
        return "V" + m[1] + SCALARS[m[2]]
    return SCALARS[text]

def main():
    OUT.mkdir(parents=True, exist_ok=True)
    names = sorted(set(re.findall(r"\b__builtin_ia32_\w+", "\n".join(p.read_text() for p in (ROOT/"src/headers").glob("*.h")))) | {"__builtin_popcount", "__builtin_popcountll"})
    discovery = OUT/"discover.c"
    discovery.write_text("void f(void) {\n" + "\n".join(n+"();" for n in names) + "\n}\n")
    result = run(discovery,"-Xclang","-ast-dump=json","-Xclang","-ast-dump-filter=__builtin_")
    nodes = {n["name"]:n for n in json_nodes(result.stdout) if n.get("kind")=="FunctionDecl"}
    assert set(names) == nodes.keys(), set(names)-nodes.keys()
    types = {}
    specs = []
    for name in names:
        node = nodes[name]
        params = [n["type"]["qualType"] for n in node.get("inner",[]) if n["kind"]=="ParmVarDecl"]
        suffix = " (" + (", ".join(params) if params else "void") + ")"
        qt = node["type"]["qualType"]
        assert qt.endswith(suffix), qt
        result = qt[:-len(suffix)]
        encodings = [encode(t) for t in [result,*params]]
        for t in [result,*params]: types.setdefault(t,"T"+str(len(types)))
        specs.append([name,result,params,encodings,[]])
    typedefs = []
    aliases = {}
    reverse = {v:k for k,v in SCALARS.items()}
    def alias(code):
        if code in aliases: return aliases[code]
        if code.startswith("P"): base = alias(code[1:])+" *"
        elif code.startswith("C"): base = "const "+alias(code[1:])
        elif code.startswith("V"):
            count=int(code[1:-1]);scalar=code[-1]
            base=reverse[scalar]
        else:base=reverse[code]
        name="U"+str(len(aliases))
        aliases[code]=name
        attr = f" __attribute__((vector_size({count*SIZES[scalar]})))" if code.startswith("V") else ""
        typedefs.append(f"typedef {base} {name}{attr};")
        return name
    for text in types: types[text]=alias(encode(text))
    prefix = "\n".join(typedefs)+"\n"
    lines = []
    cases = {}
    for idx,(name,result,params,_,_) in enumerate(specs):
        declarations = ", ".join(types[t]+" p"+str(j) for j,t in enumerate(params)) or "void"
        for j,t in enumerate(params):
            if t not in ["int","unsigned int","char","short","unsigned char","unsigned short","long long","unsigned long long"]:continue
            args = ["p"+str(k) if k==j or pt not in SCALARS or pt in ["float","double"] else "1" for k,pt in enumerate(params)]
            lines.append("void probe_"+str(idx)+"_"+str(j)+"("+declarations+") { (void)"+name+"("+", ".join(args)+"); }")
            cases[len(prefix.splitlines())+len(lines)]=(idx,j)
    path = OUT/"immediates.c";path.write_text(prefix+"\n".join(lines)+"\n")
    result = run(path)
    for m in re.finditer(r"immediates.c:(\d+):\d+: error: (.*)",result.stderr):
        if "constant integer" in m[2] or "must be a constant" in m[2]:
            idx,j = cases[int(m[1])]
            specs[idx][4].append(j)
        else: raise RuntimeError(m[0])
    range_lines=[]
    range_cases={}
    ranges={}
    for idx,(name,result,params,_,immediates) in enumerate(specs):
        declarations = ", ".join(types[t]+" p"+str(j) for j,t in enumerate(params)) or "void"
        for j in immediates:
            args=["-1" if k==j else "1" if k in immediates else "p"+str(k) for k in range(len(params))]
            range_lines.append("void range_"+str(idx)+"_"+str(j)+"("+declarations+") { (void)"+name+"("+", ".join(args)+"); }")
            range_cases[len(prefix.splitlines())+len(range_lines)]=(idx,j)
    path=OUT/"ranges.c";path.write_text(prefix+"\n".join(range_lines)+"\n")
    result=run(path)
    (OUT/"ranges.stderr").write_text(result.stderr)
    for m in re.finditer(r"ranges.c:(\d+):\d+: error: (.*)",result.stderr):
        idx,j=range_cases[int(m[1])]
        limits=re.search(r"range \[(-?\d+), ?(-?\d+)\]",m[2])
        if limits:ranges[(idx,j)]=(int(limits[1]),int(limits[2]),0)
        elif "scale" in m[2]:ranges[(idx,j)]=(1,8,278)
        else:raise RuntimeError(m[0])
    valid = []
    for idx,(name,result,params,_,immediates) in enumerate(specs):
        declarations = ", ".join(types[t]+" p"+str(j) for j,t in enumerate(params)) or "void"
        args = ["1" if j in immediates else "p"+str(j) for j in range(len(params))]
        call=name+"("+", ".join(args)+")"
        valid.append("void verify_"+str(idx)+"("+declarations+") { _Static_assert(__builtin_types_compatible_p(__typeof__("+call+"), "+types[result]+"), \"signature\"); (void)"+call+"; }")
    path=OUT/"verified.c";path.write_text(prefix+"\n".join(valid)+"\n")
    for triple in TRIPLES:
        result=run(path,"--target="+triple)
        (OUT/(triple+".stderr")).write_text(result.stderr)
        assert result.returncode==0,result.stderr[:3000]
    table=[]
    for idx,(name,_,_,codes,immediates) in enumerate(specs):
        constraints=", ".join("("+", ".join(f"{v:_}" for v in (j,*ranges.get((idx,j),(-2147483648,2147483647,0))))+")" for j in immediates)
        table.append(f'    ("{name}", "{",".join(codes)}", &[{constraints}]),')
    (ROOT/"src/translation_phases/semantic_analysis/x86_builtin_table.rs").write_text("//! Generated by `scripts/generate_x86_builtins.py` from Clang header call signatures.\n\n/// (name, result and parameter encodings, immediate index, minimum, maximum and allowed-value mask).\npub(super) type Immediate = (usize, i128, i128, u16);\n\n#[rustfmt::skip]\npub(super) const BUILTINS: &[(&str, &str, &[Immediate])] = &[\n"+"\n".join(table)+"\n];\n")
    print(f"Verified {len(specs)} signatures on all four targets; {sum(bool(s[4]) for s in specs)} have constant operands.")

if __name__ == "__main__": main()
