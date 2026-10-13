"""Tests that crash-dialog suppression reaches every child, however it is started.

Run from the repository root with
`python -m unittest discover -s scripts -p "test_crash_dialogs.py"`.
"""

import os
from pathlib import Path
import subprocess
import sys
import unittest

SCRIPTS = Path(__file__).resolve().parent

# Prints the limit flags of the innermost job of the process running it.
REPORT = ("import sys; sys.path.insert(0, sys.argv[1]); import crash_dialogs; "
          "print(crash_dialogs.job_limit_flags())")

# Suppresses dialogs, then starts REPORT children from eight threads at once,
# the pattern under which inherited error modes lose SEM_NOGPFAULTERRORBOX.
PARENT = f"""
import subprocess, sys
from concurrent.futures import ThreadPoolExecutor
sys.path.insert(0, sys.argv[1])
import crash_dialogs
crash_dialogs.suppress()
def child(_):
    return subprocess.run([sys.executable, "-I", "-c", {REPORT!r}, sys.argv[1]],
                          capture_output=True, text=True, check=True).stdout.strip()
with ThreadPoolExecutor(8) as pool:
    print(" ".join(pool.map(child, range(32))))
"""


@unittest.skipUnless(os.name == "nt", "crash dialogs are Windows-only")
class SuppressTests(unittest.TestCase):
    def test_concurrently_started_children_die_on_unhandled_exceptions(self):
        output = subprocess.run([sys.executable, "-I", "-c", PARENT, str(SCRIPTS)],
                                capture_output=True, text=True, check=True).stdout.split()
        self.assertEqual(len(output), 32)
        for flags in output:
            self.assertTrue(int(flags) & 0x400, f"child job limit flags {flags}")

    def test_survey_job_keeps_the_limit(self):
        import crash_dialogs
        import run_gcc_torture_execute as survey

        # The child waits on stdin so that it reports after joining the job.
        process = subprocess.Popen(
            [sys.executable, "-I", "-c", "import sys; sys.stdin.read(); " + REPORT, str(SCRIPTS)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        close_job = survey.windows_job(process)
        try:
            output, _ = process.communicate("")
        finally:
            close_job()
        self.assertEqual(process.returncode, 0)
        flags = int(output)
        self.assertTrue(flags & crash_dialogs.DIE_ON_UNHANDLED_EXCEPTION, f"survey job flags {flags}")
        self.assertTrue(flags & crash_dialogs.KILL_ON_JOB_CLOSE, f"survey job flags {flags}")


if __name__ == "__main__":
    unittest.main()
