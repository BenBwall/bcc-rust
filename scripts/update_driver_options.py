"""Freeze the option names the GCC and Clang driver personalities accept.

The personalities accept, and ignore, the warning, code-generation and
machine options their compiler knows but bcc does not implement, and diagnose
the rest as that compiler does. GCC's names come from a GCC on PATH
(`gcc --help=...`); Clang's from the repository's pinned Clang
(`clang --autocomplete=...`). Run with --check to compare the frozen lists.

The `cl` options clang-cl knows decide which arguments starting with `/` are
paths: clang-cl reads an argument that names none of them as an input file.
`--autocomplete` does not list them, so they come from the pinned Clang's
`clang/include/clang/Options/Options.td` (at the commit `clang --version`
names), given with --options-td.

Clang's front end reads the warning groups from its own table, which says
which groups hold warnings and so which it knows and may suggest for a
misspelled one. They come from the pinned Clang's generated
`DiagnosticGroups.inc`, given with --diagnostic-groups.

clang-cl ignores, with a warning, every GCC and Clang option its driver does
not take. --clang-cl-unknown asks the pinned Clang's clang-cl about each
option the GCC and Clang lists name, and --icx-cl asks the icx-cl given; they
write only those lists. Their options come from the frozen GCC and Clang
lists, GCC's other `--help` options and Clang's `--autocomplete=-`.

clang-cl hands its `/clang:` options to Clang's own option table, where an
option may take its value from the next arguments. --clang-driver asks the
pinned Clang's driver about every option `--autocomplete=-`, GCC's `--help`
and the `Options.td` given with --options-td name, and writes the options it
takes besides its `-W`, `-f` and `-m` flags, and those that take a separate
value; it writes only that list.
"""
import argparse
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src" / "cli" / "personality" / "options"
CLANG = ROOT / "target" / "llvm" / "bin" / ("clang.exe" if sys.platform == "win32" else "clang")
OPTION = re.compile(r"^  (-[^\s,<]+)")


def gcc_options(gcc):
    names = set()
    for category in ("warnings", "common", "optimizers", "c", "target", "undocumented"):
        result = subprocess.run([gcc, f"--help={category}"], capture_output=True, text=True, check=True)
        for line in result.stdout.splitlines():
            match = OPTION.match(line)
            if match:
                names.add(match.group(1))
    return names


def clang_options():
    names = set()
    for prefix in ("-W", "-f", "-m"):
        result = subprocess.run([str(CLANG), f"--autocomplete={prefix}"], capture_output=True, text=True, check=True)
        for line in result.stdout.splitlines():
            name = line.split("\t", 1)[0].strip()
            # `-Wno-` is the spelling that turns a warning off, not a
            # family of options: Clang warns about `-Wno-bogus` as about
            # `-Wbogus`, which CMake's `check_c_compiler_flag` relies on.
            if name.startswith(prefix) and len(name) > 2 and name != "-Wno-":
                names.add(name)
    return names


# A `cl` option definition: one of the `CL...` classes, which take both `/`
# and `-`, with its name and visibilities, or a plain `Option<["/", "-"], ...>`
# whose visibilities follow in the definition.
CL_CLASS = re.compile(
    r'def\s*\w*\s*:\s*CL(?:Compile|Ignored)?(Flag|Joined|JoinedOrSeparate|RemainingArgsJoined)'
    r'<"([^"]*)"(?:,\s*\[([^\]]*)\])?>'
)
CL_OPTION = re.compile(r'def\s*\w*\s*:\s*Option<\["/",\s*"-"\],\s*"([^"]*)",\s*KIND_(\w+)>')
CL_KINDS = {
    "Flag": "flag",
    "FLAG": "flag",
    "Joined": "joined",
    "JOINED": "joined",
    "RemainingArgsJoined": "joined",
    "REMAINING_ARGS_JOINED": "joined",
    "JoinedOrSeparate": "separate",
    "JOINED_OR_SEPARATE": "separate",
}


def clang_cl_options(options_td):
    """The `cl` options clang-cl knows, as `KIND NAME` lines: a `flag` is the
    name alone, a `joined` option takes its value joined, and a `separate`
    one takes it joined or, after the bare name, as the next argument."""
    text = Path(options_td).read_text()
    options = set()
    for definition in re.split(r"(?m)^(?=def |class |multiclass |let |defm )", text):
        match = CL_CLASS.match(definition)
        if match:
            kind, name, visibility = match.group(1), match.group(2), match.group(3) or "CLOption"
        else:
            match = CL_OPTION.match(definition)
            if not match:
                continue
            name, kind = match.group(1), match.group(2)
            visibility = re.search(r"Visibility<\[([^\]]*)\]>", definition)
            visibility = visibility.group(1) if visibility else ""
        if "CLOption" in (part.strip() for part in visibility.split(",")):
            options.add((CL_KINDS[kind], name))
    lines = sorted(options, key=lambda option: (option[1], option[0]))
    return (
        "# pinned Clang's clang-cl options (Options.td); regenerate with "
        "scripts/update_driver_options.py --options-td\n" + "".join(f"{kind} {name}\n" for kind, name in lines)
    )


# An option of a group the pinned Clang's driver ignores, and an alias
# defined without a name of its own (`-ftree-vectorize` for `-fvectorize`),
# which `--autocomplete` lists neither of.
IGNORED_GROUP = re.compile(r"Group<clang_ignored_(?:gcc_optimization_f|f|m|legacy_options)_Group>")
IGNORED_OPTION = re.compile(r'def\s*\w*\s*:\s*(Flag|Joined)<\["-"\],\s*"([^"]*)">')
IGNORED_BOOLEAN = re.compile(r'defm\s*\w*\s*:\s*BooleanFFlag<"([^"]*)">')
ALIAS = re.compile(r'def\s*:\s*Flag<\["-"\],\s*"([fm][^"]*)">,\s*Alias<(\w+)>')
# How the pinned Clang reports an option it ignores (Clang.cpp,
# `Clang::ConstructJob`).
IGNORED_REPORTS = (
    ("warn", re.compile(r"optimization flag '(.*)' is not supported \[-Wignored-optimization-argument\]")),
    ("deprecated", re.compile(r"the flag '(.*)' has been deprecated and will be ignored \[-Wunused-command-line-argument\]")),
)


def clang_ignored_options(options_td):
    """The options the pinned Clang's driver takes and ignores, which its
    `--autocomplete` does not list, as `KIND PATTERN [TARGET]` lines: those
    of its ignored groups in `Options.td`, which it reports as `warn` (an
    unsupported GCC optimization flag), `deprecated` (a retired flag) or
    `ignore` (nothing), and as `alias` the unnamed aliases of a `-f` or `-m`
    flag it lists, the TARGET they stand for. A joined option's PATTERN ends
    in `*`. The driver itself classifies each, so an option it refuses is
    left out."""
    import tempfile

    text = Path(options_td).read_text()
    listed = set()
    for kind in ("flags", "machine"):
        listed.update(line for line in (OUT / f"clang-{kind}.list").read_text().splitlines() if not line.startswith("#"))
    candidates = {}
    for definition in re.split(r"(?m)^(?=def |class |multiclass |let |defm )", text):
        alias = ALIAS.match(definition)
        if alias:
            target = "-" + alias.group(2).replace("_EQ", "=").replace("_", "-")
            if target in listed:
                candidates["-" + alias.group(1)] = ("alias", target)
            continue
        if not IGNORED_GROUP.search(definition):
            continue
        option = IGNORED_OPTION.match(definition)
        boolean = IGNORED_BOOLEAN.match(definition)
        if option:
            joined = option.group(1) == "Joined"
            candidates["-" + option.group(2)] = ("joined" if joined else "flag", None)
        elif boolean:
            for name in (f"-f{boolean.group(1)}", f"-fno-{boolean.group(1)}"):
                candidates[name] = ("flag", None)
    lines = []
    with tempfile.TemporaryDirectory() as scratch:
        directory = Path(scratch)
        (directory / "a.c").write_text("int main(void) { return 0; }\n")
        for name, (kind, target) in sorted(candidates.items()):
            argument = f"{name}1" if kind == "joined" else name
            result = subprocess.run(
                [str(CLANG), "-c", str(directory / "a.c"), "-o", str(directory / "a.o"), argument],
                capture_output=True,
                text=True,
                cwd=directory,
            )
            if result.returncode != 0:
                continue
            pattern = f"{name}*" if kind == "joined" else name
            if kind == "alias":
                if not result.stderr:
                    lines.append(f"alias {pattern} {target}")
                continue
            report = "ignore" if not result.stderr else None
            for label, message in IGNORED_REPORTS:
                match = message.search(result.stderr)
                if match and match.group(1) == argument:
                    report = label
            if report:
                lines.append(f"{report} {pattern}")
    lines.sort(key=lambda line: line.split(" ")[1])
    return (
        "# pinned Clang's ignored options and unnamed aliases (Options.td); regenerate with "
        "scripts/update_driver_options.py --options-td\n" + "".join(f"{line}\n" for line in lines)
    )


DIAG_ENTRY = re.compile(
    r"DIAG_ENTRY\(\w+ /\* (.*?) \*/, \d+, (?:/\* (DiagArray\d+) \*/ )?\d+, "
    r"(?:/\* (DiagSubGroup\d+) \*/ )?\d+, R\""
)
DIAG_ARRAY = re.compile(r"/\* (DiagArray\d+|DiagSubGroup\d+) \*/ ([^\n]*?)-1,")


def clang_warning_groups(groups_inc):
    """The warning groups of the pinned Clang's front end, in its table's
    order, as lines for `known.rs`: a group Clang may suggest for a
    misspelled one, or with `~` one it knows but never suggests, as it
    suggests no group without warnings (`DiagnosticIDs::getNearestOption`).
    A group of remarks alone is no warning option, and the unnamed group
    the table begins with, which caps the suggestions' distance, is left
    out."""
    text = Path(groups_inc).read_text()
    arrays = {
        match.group(1): [item.strip() for item in match.group(2).split(",") if item.strip()]
        for match in DIAG_ARRAY.finditer(text)
    }
    entries = [match.groups() for match in DIAG_ENTRY.finditer(text)]

    def warnings(index):
        # `getDiagnosticsInGroup` for warnings: whether none was found
        # (an empty group counts as found), and how many there are.
        _, members, subgroups = entries[index]
        if not members and not subgroups:
            return False, 0
        missing, count = True, 0
        for diagnostic in arrays.get(members, []):
            if not diagnostic.startswith("diag::remark_"):
                missing, count = False, count + 1
        for subgroup in arrays.get(subgroups, []):
            sub_missing, sub_count = warnings(int(subgroup))
            missing, count = missing and sub_missing, count + sub_count
        return missing, count

    lines = []
    for index, (name, _, _) in enumerate(entries):
        missing, count = warnings(index)
        if missing or not name:
            continue
        lines.append(name if count else f"~{name}")
    return (
        "# pinned Clang's warning groups (DiagnosticGroups.inc), `~` for those it never suggests; "
        "regenerate with scripts/update_driver_options.py --diagnostic-groups\n"
        + "".join(f"{line}\n" for line in lines)
    )


UNKNOWN = re.compile(
    r"unknown argument ignored in clang-cl(?:: '(?P<plain>.*)'|"
    r" '(?P<named>.*)'; did you mean '(?P<nearest>.*)'\?) \[-Wunknown-argument\]"
)


def unknown_to_cl(driver, directory, argument):
    """Whether the clang-cl `driver` ignores `argument` as unknown, as
    (ignored, suggestion), reading its first warning about it."""
    result = subprocess.run(
        driver + ["-###", "/c", str(directory / "a.c"), argument],
        capture_output=True,
        text=True,
        cwd=directory,
    )
    for line in result.stderr.splitlines():
        match = UNKNOWN.search(line)
        if match and argument in (match.group("plain"), match.group("named")):
            return True, match.group("nearest")
    return False, None


def joins_value(name, directory):
    """Whether Clang's own driver reads `name` followed by text as one
    option, as it does a joined option's value (`-lfoo`, `-isystemdir`)."""
    result = subprocess.run(
        [str(CLANG), "-###", "-c", str(directory / "a.c"), f"{name}Zq"],
        capture_output=True,
        text=True,
        cwd=directory,
    )
    return f"unknown argument: '{name}Zq'" not in result.stderr


def cl_unknown_options(driver, label, option, candidates):
    """The GCC and Clang options the clang-cl `driver` ignores with its
    warning `unknown argument ignored in clang-cl` (Clang's
    `Driver::ParseArgStrings`), as `KIND NAME [NEAREST]` lines.

    A `flag` is ignored as spelled, a `joined` option also with text after
    it, which an option ending in `=` needs (its value), and NEAREST is
    the option the warning suggests for the spelling itself, or with `+`
    the one it suggests, followed by the same value, for a joined option
    given one. A `known` line is an option the driver takes although a
    joined entry is a prefix of its name, which therefore does not cover
    it. clang-cl takes every `-W` option, so none is asked."""
    import concurrent.futures
    import tempfile

    with tempfile.TemporaryDirectory() as scratch:
        directory = Path(scratch)
        (directory / "a.c").write_text("int main(void) { return 0; }\n")
        names = sorted(name for name in candidates if not name.startswith("-W"))

        def survey(name):
            if name.endswith("="):
                ignored, nearest = unknown_to_cl(driver, directory, f"{name}1")
                if not ignored:
                    return name, None
                # The suggestion keeps the value: `-mabs=1` draws `-mabi=1`.
                if nearest is not None:
                    assert nearest.endswith("=1"), (name, nearest)
                    nearest = "+" + nearest[:-1]
                return name, ("joined", nearest)
            ignored, nearest = unknown_to_cl(driver, directory, name)
            if not ignored:
                return name, None
            kind = "flag"
            if joins_value(name, directory):
                joined, joined_nearest = unknown_to_cl(driver, directory, f"{name}Zq")
                if joined and joined_nearest is None:
                    kind = "joined"
            return name, (kind, nearest)

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            results = dict(pool.map(survey, names))
    ignored = {name: entry for name, entry in results.items() if entry}
    joined = [name for name, (kind, _) in ignored.items() if kind == "joined"]
    known = sorted(
        name
        for name, entry in results.items()
        if entry is None and any(name.startswith(prefix) and name != prefix for prefix in joined)
    )
    lines = [
        f"{kind} {name}" + (f" {nearest}" if nearest else "") for name, (kind, nearest) in sorted(ignored.items())
    ]
    lines += [f"known {name}" for name in known]
    lines.sort(key=lambda line: line.split(" ")[1])
    return (
        f"# {label}: the GCC and Clang options it ignores as unknown; regenerate with "
        f"scripts/update_driver_options.py {option}\n" + "".join(f"{line}\n" for line in lines)
    )


MISSING_VALUE = re.compile(r"argument to '(?P<name>.*)' is missing \(expected (?P<count>\d+) values?\)")
TD_SPELLING = re.compile(r'<\[((?:"-{1,2}",?\s*)+)\],\s*"([^"]+)"')
UNUSED = re.compile(r"argument unused during compilation: '(.*)' \[-Wunused-command-line-argument\]")
# The kinds of step clang-cl runs, each as the tag that says an option is
# unused in it and the options that ask for it, of which every one must
# leave the option unused: only compiling, preprocessing, only checking,
# and linking.
UNUSED_STEPS = (
    ("unused-c", (["/c"],)),
    ("unused-E", (["/E"], ["/EP"], ["/P"])),
    ("unused-Zs", (["/Zs"],)),
    ("unused-link", ([],)),
)


def clang_driver_options(gcc, options_td):
    """The options the pinned Clang's own driver takes (Clang's option table,
    as `Driver::ParseArgStrings` reads it without clang-cl's visibility), as
    `KIND NAME` lines, besides the `-W`, `-f` and `-m` flags the other lists
    name: a `flag` is the name alone, a `joined` option takes text after its
    name, and `separate N` names one whose bare name takes the next N
    arguments as its values, which the `-f` and `-m` options among them are
    listed for too. An option that is both is listed twice. A line ends in
    `cl` where clang-cl's driver also takes the option bare, outside
    `/clang:`, as its mode shows it, and does not ignore it as unknown, and
    in `unused-c`, `unused-E`, `unused-Zs` and `unused-link` where clang-cl
    warns that the option, given after `/clang:`, is unused during
    compilation in that kind of step (`/c`, each of `/E`, `/EP` and `/P`,
    `/Zs`, a link), with its values or, joined, both alone and with one. The
    candidates are `--autocomplete=-`, which leaves out the aliases
    (`-target`), the spellings in `Options.td` and GCC's options."""
    import concurrent.futures
    import tempfile

    result = subprocess.run([str(CLANG), "--autocomplete=-"], capture_output=True, text=True, check=True)
    names = {line.split("\t", 1)[0].strip() for line in result.stdout.splitlines()}
    for match in TD_SPELLING.finditer(Path(options_td).read_text()):
        for prefix in re.findall(r'"(-{1,2})"', match.group(1)):
            names.add(prefix + match.group(2))
    names.update(gcc_options(gcc))
    names = sorted(
        name
        for name in names
        if name.startswith("-")
        and len(name) > 1
        and not name.startswith("-W")
        and not name.startswith(("-cc1", "--driver-mode"))
        and " " not in name
    )

    with tempfile.TemporaryDirectory() as scratch:
        directory = Path(scratch)
        (directory / "a.c").write_text("int main(void) { return 0; }\n")

        def unknown(argument):
            result = subprocess.run(
                [str(CLANG), "-###", "-c", str(directory / "a.c"), argument],
                capture_output=True,
                text=True,
                cwd=directory,
                timeout=60,
            )
            refused = (f"unknown argument: '{argument}'", f"unknown argument '{argument}';")
            return any(text in result.stderr for text in refused), result.stderr

        def unused_in(step, words):
            """Whether clang-cl calls the `/clang:` option `words` unused in
            the step `step` asks for (`/c`, `/E`, none to link): any
            `argument unused during compilation` but about the step's own
            option, which an option that changes the step leaves unused
            (`/c` beside `-S`), in whatever spelling Clang renders the
            option (`'-T Zq'` for `-TZq`)."""
            result = subprocess.run(
                [str(CLANG), "--driver-mode=cl", "-###", *step, str(directory / "a.c")]
                + [f"/clang:{word}" for word in words],
                capture_output=True,
                text=True,
                cwd=directory,
                timeout=60,
            )
            return any(match.group(1) not in step for match in UNUSED.finditer(result.stderr))

        def cl_tags(line, separate):
            kind, name, *count = line.split(" ")
            argument = f"{name}Zq" if kind == "joined" else name
            ignored, _ = unknown_to_cl([str(CLANG), "--driver-mode=cl"], directory, argument)
            if not ignored:
                line += " cl"
            # A joined option is measured with a value and without one
            # (`-dD`, which `-d` absorbs with a value), unless its bare name
            # takes its value from the next argument instead (`-T`).
            if kind == "joined":
                forms = [[argument]] if separate else [[name], [argument]]
            else:
                forms = [[argument] + ["x"] * int(count[0] if count else 0)]
            for tag, steps in UNUSED_STEPS:
                if all(unused_in(step, words) for step in steps for words in forms):
                    line += f" {tag}"
            return line

        def survey(name):
            refused, stderr = unknown(name)
            lines = []
            for match in MISSING_VALUE.finditer(stderr):
                if match.group("name") == name:
                    lines.append(f"separate {name} {match.group('count')}")
            if not name.startswith(("-f", "-m")):
                if not lines and not refused:
                    lines.append(f"flag {name}")
                if not refused or lines:
                    joined, _ = unknown(f"{name}Zq")
                    if not joined:
                        lines = [line for line in lines if not line.startswith("flag ")]
                        lines.append(f"joined {name}")
            separate = any(line.startswith("separate ") for line in lines)
            return [cl_tags(line, separate) for line in lines]

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            results = list(pool.map(survey, names))
    lines = sorted({line for entry in results for line in entry}, key=lambda line: (line.split(" ")[1], line))
    return (
        "# pinned Clang's driver: its options besides the -W, -f and -m flags, and those taking separate values; "
        "regenerate with scripts/update_driver_options.py --clang-driver --options-td\n"
        + "".join(f"{line}\n" for line in lines)
    )


def unknown_option_candidates(gcc):
    """Every option name GCC or the pinned Clang lists, a value cut off at
    its `=`: the frozen `-f` and `-m` lists, GCC's other options from its
    `--help` (which vary little between its versions) and Clang's
    `--autocomplete`, and each `-f` and `-m` flag's other form."""
    names = set()
    for kind in ("flags", "machine"):
        for compiler in ("gcc", "clang"):
            lines = (OUT / f"{compiler}-{kind}.list").read_text().splitlines()
            names.update(line for line in lines if not line.startswith("#"))
    names.update(name for name in gcc_options(gcc) if not name.startswith(("-f", "-m", "-W")))
    result = subprocess.run([gcc, "--help"], capture_output=True, text=True, check=True)
    for line in result.stdout.splitlines():
        match = OPTION.match(line)
        if match:
            names.add(match.group(1))
    result = subprocess.run([str(CLANG), "--autocomplete=-"], capture_output=True, text=True, check=True)
    names.update(line.split("\t", 1)[0].strip() for line in result.stdout.splitlines())
    cut = set()
    for name in names:
        name = re.sub(r"\[.*", "", name)
        name = name.split("=", 1)[0] + "=" if "=" in name else name
        if name.startswith("-") and len(name) > 1 and name != "--driver-mode=":
            cut.add(name)
    # GCC lists most of its `-f` and `-m` flags in one form only; the other
    # is as much an option (`-fno-strict-volatile-bitfields`).
    for name in list(cut):
        prefix = name[:2]
        if prefix in ("-f", "-m") and not name.endswith(("=", "-")):
            rest = name[2:]
            cut.add(prefix + (rest[3:] if rest.startswith("no-") else "no-" + rest))
    return cut


def render(names, prefix, source):
    selected = sorted(name for name in names if name.startswith(prefix) and len(name) > len(prefix))
    return f"# {source}; regenerate with scripts/update_driver_options.py\n" + "".join(f"{name}\n" for name in selected)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compare instead of writing")
    parser.add_argument("--gcc", default="gcc", help="the GCC whose options to freeze")
    parser.add_argument("--options-td", help="the pinned Clang's clang/include/clang/Options/Options.td")
    parser.add_argument("--diagnostic-groups", help="the pinned Clang's include/clang/Basic/DiagnosticGroups.inc")
    parser.add_argument(
        "--clang-cl-unknown",
        action="store_true",
        help="survey the options the pinned Clang's clang-cl ignores as unknown",
    )
    parser.add_argument("--icx-cl", help="an icx-cl to survey the same way, as --clang-cl-unknown does")
    parser.add_argument(
        "--clang-driver",
        action="store_true",
        help="survey the options the pinned Clang's driver takes (needs --options-td)",
    )
    args = parser.parse_args()
    files = {}
    if args.clang_driver:
        if not args.options_td:
            parser.error("--clang-driver needs --options-td")
        files["clang-driver.list"] = clang_driver_options(args.gcc, args.options_td)
    elif args.clang_cl_unknown or args.icx_cl:
        # A survey of its own, against the frozen GCC and Clang lists.
        candidates = unknown_option_candidates(args.gcc)
        if args.clang_cl_unknown:
            files["clang-cl-unknown.list"] = cl_unknown_options(
                [str(CLANG), "--driver-mode=cl"], "pinned Clang's clang-cl", "--clang-cl-unknown", candidates
            )
        if args.icx_cl:
            files["icx-cl-unknown.list"] = cl_unknown_options([args.icx_cl], "icx-cl 2026.2.0", "--icx-cl", candidates)
    else:
        version = subprocess.run(
            [args.gcc, "-dumpfullversion"], capture_output=True, text=True, check=True
        ).stdout.strip()
        gcc = gcc_options(args.gcc)
        clang = clang_options()
        for prefix, kind in (("-W", "warnings"), ("-f", "flags"), ("-m", "machine")):
            files[f"gcc-{kind}.list"] = render(gcc, prefix, f"GCC {version} --help")
            files[f"clang-{kind}.list"] = render(clang, prefix, "pinned Clang --autocomplete")
        if args.options_td:
            files["clang-cl.list"] = clang_cl_options(args.options_td)
            files["clang-ignored.list"] = clang_ignored_options(args.options_td)
        if args.diagnostic_groups:
            files["clang-warning-groups.list"] = clang_warning_groups(args.diagnostic_groups)
    stale = []
    OUT.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        path = OUT / name
        if args.check:
            old = path.read_text() if path.exists() else ""
            # The GCC version line may differ between hosts; compare names.
            if old.splitlines()[1:] != text.splitlines()[1:]:
                stale.append(name)
        else:
            path.write_text(text, newline="\n")
    if stale:
        print("stale:", ", ".join(stale))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
