"""Regression tests for the libc header survey and sysroot fetcher.

Run from the repository root with `python -m unittest discover -s scripts -p 'test_*.py'`.
"""

import os
from pathlib import Path
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from fetch_libc_sysroots import ar_member, plan, safe_name
from libc_header_survey import (
    DEFAULT_BCC_ARGS, bcc_result, clang_status, configurations, expand_template, survey_row,
)


def result(output, exit_code=0):
    return {"exit": exit_code, "timeout": False, "stdout": "", "stderr": output}


def member(name, kind=tarfile.REGTYPE, linkname=""):
    info = tarfile.TarInfo(name)
    info.type = kind
    info.linkname = linkname
    return info


class TemplateTests(unittest.TestCase):
    values = {"triple": "x86_64-pc-windows-msvc", "sysroot": "", "std": "c17", "config": "msvc"}

    def test_include_placeholder_repeats_include_pairs(self):
        arguments = expand_template('--std={std} "--isystem {dir}"', self.values,
                                    ["C:/Program Files/a", "b"], "t.c")
        self.assertEqual(arguments, ["--std=c17", "--isystem", "C:/Program Files/a",
                                     "--isystem", "b", "t.c"])

    def test_empty_placeholder_drops_its_word(self):
        arguments = expand_template("--target={triple} --sysroot={sysroot} {input} -x", self.values,
                                    [], "t.c")
        self.assertEqual(arguments, ["--target=x86_64-pc-windows-msvc", "t.c", "-x"])

    def test_unknown_placeholder_is_rejected(self):
        with self.assertRaises(ValueError):
            expand_template("--prefix={prefix}", self.values, [], "t.c")


class SurveyCommandTests(unittest.TestCase):
    def test_default_commands_use_each_configurations_target_and_extensions(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "survey fixtures"
            include_dirs = [root / "include one", root / "include two"]
            for directory in include_dirs:
                directory.mkdir(parents=True)
            (include_dirs[0] / "stdio.h").write_text("", encoding="utf-8")
            for name in ("glibc-x86_64-linux", "musl-x86_64-linux"):
                sysroot = root / "sysroots" / name
                sysroot.mkdir(parents=True)
                (sysroot / "sysroot.json").write_text("{}", encoding="utf-8")
            args = SimpleNamespace(
                sysroots=root / "sysroots", gcc="fixture-gcc", clang="fixture-clang",
                bcc="fixture-bcc", output=root / "output", timeout=5,
                bcc_args=DEFAULT_BCC_ARGS, bcc_args_for={},
            )
            environment = {"INCLUDE": os.pathsep.join(map(str, include_dirs))}
            gcc_results = [
                result(""),
                result(""),
                result(""),
            ]
            gcc_results[0]["stdout"] = "x86_64-w64-mingw32\n"
            gcc_results[1]["stderr"] = (
                "#include <...> search starts here:\n"
                + "".join(f" {directory}\n" for directory in include_dirs)
                + "End of search list.\n"
            )
            gcc_results[2]["stdout"] = "fixture GCC\n"
            with patch("libc_header_survey.shutil.which", return_value=str(root / "bin/gcc")), \
                    patch("libc_header_survey.invoke", side_effect=gcc_results):
                selected = {
                    name: factory()[0]
                    for name, factory in configurations(args, root / "resource", environment).items()
                }
            expected = {
                "glibc-x86_64-linux": ("x86_64-unknown-linux-gnu",
                                       [f"--sysroot={root / 'sysroots' / 'glibc-x86_64-linux'}"]),
                "musl-x86_64-linux": ("x86_64-unknown-linux-musl",
                                      [f"--sysroot={root / 'sysroots' / 'musl-x86_64-linux'}"]),
                "mingw-w64": ("x86_64-w64-windows-gnu", []),
                "msvc-ucrt": ("x86_64-pc-windows-msvc", ["-fms-extensions"]),
            }
            for name, (triple, flags) in expected.items():
                for std in ("c17", "gnu17"):
                    with self.subTest(config=name, std=std):
                        configuration = selected[name]
                        self.assertIsNotNone(configuration)
                        row = {"config": name, "header": "stdio.h", "std": std,
                               "includes": ["stdio.h"]}
                        with patch("libc_header_survey.invoke", return_value=result("")) as invoke:
                            surveyed = survey_row(row, configuration, args, environment)
                        source = args.output / "tu" / name / std / "stdio.h.c"
                        clang_command, bcc_command = [call.args[0] for call in invoke.call_args_list]
                        self.assertEqual(clang_command, [
                            args.clang, *configuration["clang_flags"], f"-std={std}",
                            "-fsyntax-only", "-fno-color-diagnostics", source,
                        ])
                        self.assertEqual(bcc_command, [
                            args.bcc, f"--target={triple}", *flags, f"--std={std}",
                            *[argument for directory in configuration["include_dirs"]
                              for argument in ("-idirafter", str(directory))], str(source),
                        ])
                        self.assertEqual(surveyed["bcc"]["status"], "accepted")

    def test_global_and_configuration_templates_override_the_defaults(self):
        with tempfile.TemporaryDirectory() as temporary:
            args = SimpleNamespace(
                clang="fixture-clang", bcc="fixture-bcc", output=Path(temporary), timeout=5,
                bcc_args="--target=x86_64-unknown-linux-musl --std={std} {input}",
                bcc_args_for={},
            )
            configuration = {
                "triple": "x86_64-pc-windows-msvc", "sysroot": "", "include_dirs": [],
                "clang_flags": ["--target=x86_64-pc-windows-msvc", "-fms-extensions"],
                "bcc_flags": ["-fms-extensions"],
            }
            for override in (False, True):
                with self.subTest(configuration_override=override):
                    if override:
                        args.bcc_args_for["msvc-ucrt"] = "{flags} --std={std} {input}"
                    row = {"config": "msvc-ucrt", "header": "stdio.h", "std": "c17",
                           "includes": ["stdio.h"]}
                    with patch("libc_header_survey.invoke", return_value=result("")) as invoke:
                        survey_row(row, configuration, args, {})
                    source = str(args.output / "tu/msvc-ucrt/c17/stdio.h.c")
                    flags = ["-fms-extensions"] if override else ["--target=x86_64-unknown-linux-musl"]
                    self.assertEqual(invoke.call_args_list[1].args[0],
                                     [args.bcc, *flags, "--std=c17", source])


class ClassifierTests(unittest.TestCase):
    def test_clang_error_location_is_relative_to_its_include_directory(self):
        status = clang_status(result(
            "C:\\sys\\usr/include\\bits/x.h:3:1: warning: w [-Wfoo]\n"
            "C:\\sys\\usr/include\\stdio.h:12:5: fatal error: 'y.h' file not found\n", 1),
            ["C:/sys/usr/include"])
        self.assertEqual(status, {"status": "rejected", "errors": 1, "warnings": 1,
                                  "first_error": "'y.h' file not found",
                                  "first_error_location": "stdio.h:12"})

    def test_bcc_errors_count_despite_exit_zero(self):
        status = bcc_result(result(
            "error: unknown type name `__gnuc_va_list`\n"
            "   --> C:/sys/usr/include\\stdio.h:373:8\n"
            "1 error generated.\n"), ["C:/sys/usr/include"])
        self.assertEqual((status["status"], status["errors"], status["first_error_location"]),
                         ("diagnostics", 1, "stdio.h:373"))


class ExtractionTests(unittest.TestCase):
    def test_unsafe_member_names_are_rejected(self):
        for name in ("/usr/include/a.h", "usr/../../a.h", "usr/include/c:a.h"):
            with self.assertRaises(RuntimeError):
                safe_name(name)
        self.assertEqual(safe_name("./usr/include/a.h"), "usr/include/a.h")

    def test_links_become_copies_of_targets_inside_the_package(self):
        destinations = plan([
            member("./usr/lib/uapi/asm/unistd.h"),
            member("./usr/lib/uapi/asm", tarfile.DIRTYPE),
            member("./usr/include/x/asm/unistd.h", tarfile.SYMTYPE, "../../../lib/uapi/asm/unistd.h"),
            member("./usr/include/asm", tarfile.SYMTYPE, "../lib/uapi/asm"),
            member("./usr/include/b.h", tarfile.LNKTYPE, "./usr/lib/uapi/asm/unistd.h"),
            member("./usr/lib/libc.so", tarfile.SYMTYPE, "/lib/libc.so.6"),
        ])
        self.assertEqual(sorted(destinations["usr/lib/uapi/asm/unistd.h"]),
                         ["usr/include/asm/unistd.h", "usr/include/b.h", "usr/include/x/asm/unistd.h"])

    def test_links_leaving_the_sysroot_are_rejected(self):
        for target in ("../../../../etc/passwd", "/etc/passwd"):
            with self.assertRaises(RuntimeError):
                plan([member("usr/include/a.h", tarfile.SYMTYPE, target)])

    def test_ar_member_finds_package_data(self):
        def entry(name, data):
            header = f"{name:<16}{0:<12}{0:<6}{0:<6}{644:<8}{len(data):<10}`\n".encode()
            return header + data + (b"\n" if len(data) % 2 else b"")
        archive = b"!<arch>\n" + entry("debian-binary", b"2.0\n") + entry("data.tar.xz", b"abc")
        self.assertEqual(ar_member(archive, "data.tar"), ("data.tar.xz", b"abc"))


if __name__ == "__main__":
    unittest.main()
