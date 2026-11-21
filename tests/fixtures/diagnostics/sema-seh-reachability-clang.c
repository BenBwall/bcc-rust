// Clang's control-flow graph has no edge into an __except handler and holds
// no filter, and Clang builds none for a function with __finally (Clang
// 23.1.5, --target=x86_64-pc-windows-msvc -fms-extensions -fsyntax-only
// -Wall -Wextra).
int work(int);

// Nothing in the handler runs, so it draws no warning about code that runs;
// the code after the statement does.
int handler(int p, unsigned u, unsigned char c) {
    __try {
        work(p);
    } __except (1) {
        if (p < u)
            work(1);
        work(1 << 40);
        work(p / 0);
        if (c > 300)
            work(2);
        __try {
            work(3);
        } __except (1) {
            work(1 << 41);
        }
    }
    return p < u;
}

// A filter's conversions and comparisons go unchecked, as it is no full
// expression, but its shift is checked where it is.
int filter(int p, unsigned u) {
    __try {
        work(p);
    } __except (p < u ? 1 : 0) {
    }
    __try {
        work(p);
    } __except (1 << 40) {
    }
    return 0;
}

// A return in the handler leaves the end reachable from the guarded block
// alone, so the end is reached on every path.
int falls_through(int p) {
    work(p);
    __try {
        work(p);
    } __except (1) {
        return 2;
    }
}

int leaves(int p) {
    __try {
        if (p)
            __leave;
        return 1;
    } __except (1) {
        return 2;
    }
}

// The guarded block returns and the handler is unreachable.
int returns(int p) {
    __try {
        return work(p);
    } __except (1) {
    }
    work(p);
}

// With __finally, Clang has no graph: it warns in code that does not run
// and says nothing of the end.
int finally(int p, unsigned u) {
    __try {
        work(p);
    } __finally {
        work(1 << 40);
    }
    return 0;
    if (p < u)
        work(3);
}

int finally_end(int p) {
    while (1) {
        __try {
            if (p)
                break;
        } __finally {
        }
    }
}
