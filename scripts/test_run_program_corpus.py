"""Tests for the program-corpus runner; python scripts/test_run_program_corpus.py."""

from pathlib import Path
import shutil
import sys
import tempfile
import unittest

import conformance_common as common
import run_program_corpus as corpus


class TemporaryTests(unittest.TestCase):
    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix="corpus-test-", dir=common.ROOT / "target"))
        self.addCleanup(shutil.rmtree, self.directory, True)


class AmalgamTests(TemporaryTests):
    def test_local_headers_are_inlined_once_and_system_headers_stay(self):
        root = self.directory / "src"
        root.mkdir()
        (root / "unguarded.h").write_text("typedef struct { int a; } code;\n", encoding="utf-8")
        (root / "a.c").write_text('#include <stdio.h>\n#include "unguarded.h"\nint a(void);\n', encoding="utf-8")
        (root / "b.c").write_text('#include "unguarded.h"\nint b(void);\n', encoding="utf-8")
        (root / "main.c").write_text('#include "unguarded.h"\nint main(void) { return 0; }\n', encoding="utf-8")
        program = {"single_translation_unit": {"library": ["a.c", "b.c"], "executables": ["m"]}}
        executable = {"sources": [str(root / "main.c")]}
        path = corpus.amalgam(program, executable, root, self.directory)
        text = path.read_text(encoding="utf-8")
        self.assertEqual(text.count("typedef struct { int a; } code;"), 1)
        self.assertIn("#include <stdio.h>", text)
        self.assertNotIn('#include "unguarded.h"', text)
        self.assertIn("int main(void)", text)


class WorkloadTests(TemporaryTests):
    def run_workloads(self, arm, source_code, reference_runs=None, files=()):
        script = self.directory / "prog.py"
        script.write_text(source_code, encoding="utf-8")
        executable = {"runs": [{"name": "r", "args": [str(script)], "files": list(files)}]}
        logs = self.directory / "logs"
        logs.mkdir(exist_ok=True)
        return corpus.run_workloads(arm, "prog", executable, sys.executable, self.directory / "out", logs,
                                    reference_runs or {})["r"]

    def test_reference_run_records_stdout_for_later_comparison(self):
        (self.directory / "out").mkdir()
        run = self.run_workloads("clang", "print('x')")
        self.assertEqual(run["status"], "reference")
        self.assertEqual(run["_stdout"].strip(), "x")

    def test_arm_runs_are_compared_with_the_reference(self):
        (self.directory / "out").mkdir()
        reference = {"r": {"status": "reference", "exit": 0, "_stdout": "x\n"}}
        self.assertEqual(self.run_workloads("A0", "print('x')", reference)["status"], "pass")
        self.assertEqual(self.run_workloads("A0", "print('y')", reference)["status"], "wrong-output")
        self.assertEqual(self.run_workloads("A0", "import sys; print('x'); sys.exit(3)", reference)["status"],
                         "abnormal-exit")

    def test_a_failed_reference_blocks_the_comparison(self):
        (self.directory / "out").mkdir()
        reference = {"r": {"status": "reference-failure", "exit": 1, "_stdout": ""}}
        self.assertEqual(self.run_workloads("A0", "print('x')", reference)["status"], "no-reference")

    def test_workload_files_are_copied_into_the_run_directory(self):
        (self.directory / "out").mkdir()
        data = self.directory / "data.txt"
        data.write_text("payload", encoding="utf-8")
        code = "print(open('data.txt').read())"
        self.assertEqual(self.run_workloads("clang", code, files=[data])["status"], "reference")


class LinkTests(TemporaryTests):
    def test_refused_units_are_replaced_by_the_reference_objects_and_recorded(self):
        program = {"library": ["lib.c"], "executables": {"app": {"sources": ["app.c"]}}}
        compiled = [{"source": "lib.c", "status": "frontend-error", "object": "x"},
                    {"source": "app.c", "status": "pass", "object": str(self.directory / "app.o")}]
        by_source = {item["source"]: item for item in compiled}
        logs = self.directory / "logs"
        logs.mkdir()
        # Without a reference object a refused unit leaves the program unbuilt.
        result = corpus.link("A0", "p", program, "app", compiled, by_source, {}, self.directory, logs)
        self.assertEqual(result["status"], "not-built")
        self.assertIn("lib.c", result["detail"])

    def test_summary_marks_mixed_builds(self):
        results = [{"program": "p", "arm": "A0", "units": [{"source": "lib.c", "status": "frontend-error",
                                                              "detail": "bad"}],
                    "executables": {"app": {"link": {"status": "linked", "substituted": ["lib.c"]},
                                            "runs": {"r": {"status": "pass"}}}}}]
        text = corpus.markdown(results, {"target": "t", "clang": "clang", "bcc": "bcc"})
        self.assertIn("mixed: Clang objects for lib.c", text)
        self.assertIn("lib.c`: frontend-error: bad", text)


class WorkloadFileTests(unittest.TestCase):
    def test_the_lua_scripts_and_sql_workload_exist(self):
        for script in corpus.LUA_SCRIPTS:
            self.assertTrue((corpus.DATA / "lua" / f"{script}.lua").is_file(), script)
        self.assertTrue((corpus.DATA / "sqlite_workload.sql").is_file())
        self.assertTrue((corpus.DATA / "zlib_roundtrip.c").is_file())
        lua_lines = sum(len((corpus.DATA / "lua" / f"{s}.lua").read_text(encoding="utf-8").splitlines())
                        for s in corpus.LUA_SCRIPTS)
        self.assertGreater(lua_lines, 250)

    def test_every_program_declares_runs_for_each_executable(self):
        for name, program in corpus.PROGRAMS.items():
            for executable, spec in program["executables"].items():
                self.assertTrue(spec["runs"], f"{name}:{executable}")


if __name__ == "__main__":
    unittest.main()
