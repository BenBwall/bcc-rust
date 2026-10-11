"""Keep crashing child programs from opening Windows error dialogs.

Tests and benchmark harnesses run C programs that bcc compiled, and a
miscompiled or deliberately failing program can crash or abort. Windows then
shows a Windows Error Reporting dialog for each one. The error mode set here
suppresses those dialogs for this process and is inherited by every process it
starts, so calling `suppress()` once before spawning children covers the whole
tree. Elsewhere it does nothing.
"""

import os

# SetErrorMode flags (winbase.h): no critical-error, fault, or open-file boxes.
SEM_FAILCRITICALERRORS = 0x0001
SEM_NOGPFAULTERRORBOX = 0x0002
SEM_NOOPENFILEERRORBOX = 0x8000


def suppress() -> None:
    if os.name != "nt":
        return
    import ctypes

    ctypes.windll.kernel32.SetErrorMode(
        SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX
    )
