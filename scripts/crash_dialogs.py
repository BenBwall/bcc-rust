"""Keep crashing child programs from opening Windows error dialogs.

Tests and benchmark harnesses run C programs that bcc compiled, and a
miscompiled or deliberately failing program can crash or abort. Windows then
runs Windows Error Reporting for it, which opens a dialog (or, where WER cannot
run, an "Application Error" hard-error popup).

The process error mode alone does not prevent this. Children inherit it, but
Windows briefly rewrites the parent's mode while it creates a process, so a
harness that starts processes from several threads at once hands some children
a mode without SEM_NOGPFAULTERRORBOX (about one in seven with eight threads).
`suppress()` therefore also places this process in a job whose limits make
every unhandled exception terminate the faulting process at once; children
join the job when they are created, before they run. Windows applies only the
innermost job's limit, so a harness that puts a child in a job of its own must
include `DIE_ON_UNHANDLED_EXCEPTION` in that job's limit flags too.

https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information
Elsewhere this module does nothing.
"""

import os

# SetErrorMode flags (winbase.h): no critical-error, fault, or open-file boxes.
SEM_FAILCRITICALERRORS = 0x0001
SEM_NOGPFAULTERRORBOX = 0x0002
SEM_NOOPENFILEERRORBOX = 0x8000

# JOBOBJECT_BASIC_LIMIT_INFORMATION.LimitFlags (winnt.h).
DIE_ON_UNHANDLED_EXCEPTION = 0x0400
KILL_ON_JOB_CLOSE = 0x2000

JOB_OBJECT_EXTENDED_LIMIT_INFORMATION = 9

# Keeps the job handle open for the life of this process.
_job = None


def suppress() -> None:
    global _job
    if os.name != "nt":
        return
    import ctypes
    from ctypes import wintypes

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    kernel.SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX)
    if _job is None:
        _job = create_job(DIE_ON_UNHANDLED_EXCEPTION)
        # -1 is the pseudo-handle for the current process.
        if not kernel.AssignProcessToJobObject(_job, wintypes.HANDLE(-1)):
            raise ctypes.WinError(ctypes.get_last_error())


def create_job(flags):
    """Return a new anonymous job handle with the given basic limit flags."""
    import ctypes
    from ctypes import wintypes

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    kernel.CreateJobObjectW.restype = wintypes.HANDLE
    kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                               wintypes.DWORD]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    job = kernel.CreateJobObjectW(None, None)
    if not job:
        raise ctypes.WinError(ctypes.get_last_error())
    limits = ExtendedLimits()
    limits.basic.flags = flags
    if not kernel.SetInformationJobObject(job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                                          ctypes.byref(limits), ctypes.sizeof(limits)):
        error = ctypes.get_last_error()
        kernel.CloseHandle(job)
        raise ctypes.WinError(error)
    return job


def job_limit_flags():
    """Return the basic limit flags of this process's innermost job, or None."""
    import ctypes
    from ctypes import wintypes

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                                 wintypes.DWORD, ctypes.c_void_p]
    limits = ExtendedLimits()
    if not kernel.QueryInformationJobObject(None, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                                            ctypes.byref(limits), ctypes.sizeof(limits), None):
        return None
    return limits.basic.flags


def _structures():
    import ctypes
    from ctypes import wintypes

    class BasicLimits(ctypes.Structure):
        _fields_ = [("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64),
                    ("flags", wintypes.DWORD), ("min_working_set", ctypes.c_size_t),
                    ("max_working_set", ctypes.c_size_t), ("processes", wintypes.DWORD),
                    ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                    ("scheduling", wintypes.DWORD)]

    class ExtendedLimits(ctypes.Structure):
        _fields_ = [("basic", BasicLimits), ("io_counters", ctypes.c_uint64 * 6),
                    ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                    ("peak_process_memory", ctypes.c_size_t), ("peak_job_memory", ctypes.c_size_t)]

    return ExtendedLimits


if os.name == "nt":
    ExtendedLimits = _structures()
