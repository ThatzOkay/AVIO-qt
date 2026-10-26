#!/usr/bin/env python3
"""Generate a qmldir + .qmltypes for the #[qobject] types in src/, so qmllint/qmlls
can resolve `import <crate_name>` (qtbridge registers the module at runtime only).

Supports #[qobject] impl blocks with #[qslot] functions, #[qsignal] functions and qproperty!(...) entries. Output: .qml-types/<crate>/
"""
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
module = re.search(r'^name\s*=\s*"([^"]+)"', (root / "Cargo.toml").read_text(), re.M).group(1)

QT_TYPES = {
    "i8": "int", "i16": "int", "i32": "int", "u8": "int", "u16": "int", "u32": "uint",
    "i64": "qlonglong", "u64": "qulonglong", "isize": "qlonglong", "usize": "qulonglong",
    "f32": "float", "f64": "double", "bool": "bool", "String": "QString", "&str": "QString",
}

OBJ_RE = re.compile(r"#\[qobject(?:\(([^)]*)\))?\]\s*impl(?:<[^>]*>)?\s+(\w+)\s*\{", re.S)
SLOT_RE = re.compile(r"#\[qslot[^\]]*\]\s*(?:pub\s+)?fn\s+(\w+)\s*\(([^)]*)\)\s*(?:->\s*([^{]+?))?\s*\{", re.S)
SIGNAL_RE = re.compile(r"#\[qsignal[^\]]*\]\s*(?:pub\s+)?fn\s+(\w+)\s*\(([^)]*)\)")
PROP_RE = re.compile(r'qproperty!\(\s*"(\w+)"\s*((?:,[^;]*?)?)\)\s*;', re.S)


def qt_type(rust: str) -> str:
    rust = rust.strip()
    return QT_TYPES.get(rust, QT_TYPES.get(rust.lstrip("&"), "QVariant"))


def block_body(text: str, start: int) -> str:
    depth = 1
    for i in range(start, len(text)):
        depth += {"{": 1, "}": -1}.get(text[i], 0)
        if depth == 0:
            return text[start:i]
    return text[start:]


components = []
for path in sorted((root / "src").rglob("*.rs")):
    text = path.read_text()
    for m in OBJ_RE.finditer(text):
        flags, name = m.group(1) or "", m.group(2)
        methods = []
        for s in SLOT_RE.finditer(block_body(text, m.end())):
            args = [a.strip() for a in s.group(2).split(",") if a.strip()]
            args = [a for a in args if not re.fullmatch(r"&?\s*(mut\s+)?self", a)]
            params = [(a.split(":", 1)[0].strip(), qt_type(a.split(":", 1)[1])) for a in args]
            methods.append((s.group(1), params, qt_type(s.group(3)) if s.group(3) else "void"))
        body = block_body(text, m.end())
        signals = [
            (g.group(1), [(a.split(":", 1)[0].strip(), qt_type(a.split(":", 1)[1]))
                          for a in (x.strip() for x in g.group(2).split(",")) if ":" in a])
            for g in SIGNAL_RE.finditer(body)
        ]
        # Getter/setter/member types come from the fn signatures and the struct fields
        struct = re.search(r"struct\s+" + name + r"\s*\{(.*?)\}", text, re.S)
        props = []
        for pm in PROP_RE.finditer(body):
            opts = dict(
                (k.strip(), v.strip())
                for k, v in (o.split("=", 1) for o in pm.group(2).split(",") if "=" in o)
            )
            ty = "QVariant"
            read = re.search(r"fn\s+" + re.escape(opts.get("Read", "\0")) + r"\s*\([^)]*\)\s*->\s*([^{]+?)\s*\{", body)
            member = struct and re.search(r"\b" + re.escape(opts.get("Member", "\0")) + r"\s*:\s*([^,\n]+)", struct.group(1))
            if read:
                ty = qt_type(read.group(1))
            elif member:
                ty = qt_type(member.group(1))
            props.append((pm.group(1), ty, opts, "Constant" in pm.group(2)))
        components.append((name, "Singleton" in flags, methods, signals, props))

out = root / ".qml-types" / module
out.mkdir(parents=True, exist_ok=True)

lines = ["import QtQuick.tooling 1.2", "", "Module {"]
for name, singleton, methods, signals, props in components:
    lines += [
        "    Component {",
        f'        name: "{name}"',
        '        prototype: "QObject"',
        f'        exports: ["{module}/{name} 1.0"]',
        "        exportMetaObjectRevisions: [256]",
        f"        isCreatable: {str(not singleton).lower()}",
        f"        isSingleton: {str(singleton).lower()}",
    ]
    for pname, ptype, opts, constant in props:
        read = opts.get("Read", opts.get("Member", pname))
        entry = [f'name: "{pname}"', f'type: "{ptype}"', f'read: "{read}"']
        if "Write" in opts:
            entry.append(f'write: "{opts["Write"]}"')
        if "Notify" in opts:
            entry.append(f'notify: "{opts["Notify"]}"')
        if constant:
            entry.append("isConstant: true")
        lines.append("        Property { " + "; ".join(entry) + " }")
    for sname, params in signals:
        lines.append("        Signal {")
        lines.append(f'            name: "{sname}"')
        for pname, ptype in params:
            lines.append(f'            Parameter {{ name: "{pname}"; type: "{ptype}" }}')
        lines.append("        }")
    for mname, params, ret in methods:
        lines.append("        Method {")
        lines.append(f'            name: "{mname}"')
        if ret != "void":
            lines.append(f'            type: "{ret}"')
        for pname, ptype in params:
            lines.append(f'            Parameter {{ name: "{pname}"; type: "{ptype}" }}')
        lines.append("        }")
    lines.append("    }")
lines.append("}")

(out / "plugin.qmltypes").write_text("\n".join(lines) + "\n")
(out / "qmldir").write_text(f"module {module}\ntypeinfo plugin.qmltypes\n")
print(f"wrote {out} ({len(components)} type(s))", file=sys.stderr)
