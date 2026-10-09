"""Regression tests for the libc header survey and sysroot fetcher.

Run from the repository root with `python -m unittest discover -s scripts -p 'test_*.py'`.
"""

import tarfile
import unittest

from fetch_libc_sysroots import ar_member, plan, safe_name
from libc_header_survey import bcc_result, clang_status, expand_template


def result(output, exit_code=0):
    return {"exit": exit_code, "timeout": False, "stdout": "", "stderr": output}


def member(name, kind=tarfile.REGTYPE, linkname=""):
    info = tarfile.TarInfo(name)
    info.type = kind
    info.linkname = linkname
    return info


class TemplateTests(unittest.TestCase):
    values = {"triple": "x86_64-pc-windows-msvc", "sysroot": "", "std": "c17", "config": "msvc"}

    def test_default_template_repeats_include_pairs(self):
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
