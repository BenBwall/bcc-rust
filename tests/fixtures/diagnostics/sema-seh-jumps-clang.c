// Jumps into and out of SEH blocks, in Clang's words (Clang 23.1.5,
// --target=x86_64-pc-windows-msvc -fms-extensions -fsyntax-only).
int work(int);

int into_try(int p) {
    if (p)
        goto inside;
    __try {
    inside:
        work(p);
    } __finally {
        work(0);
    }
    return 0;
}

int into_handlers(int p) {
    __try {
        work(p);
    } __except (1) {
    handler:
        work(1);
    }
    __try {
        work(p);
    } __finally {
    cleanup:
        work(2);
    }
    if (p > 1)
        goto handler;
    if (p > 2)
        goto cleanup;
    return 0;
}

int nested(int p) {
    if (p)
        goto deep;
    __try {
        work(p);
    } __except (1) {
        __try {
        deep:
            work(p);
        } __finally {
        }
    }
    return 0;
}

int dispatch(int p) {
    switch (p) {
    case 0:
        __try {
    case 1:
            work(1);
        } __except (1) {
        }
        break;
    }
    return 0;
}

int within(int p) {
    int r = 0;
    __try {
        if (p)
            goto in;
        r = 3;
    in:
        r = 1;
    } __finally {
    }
    __try {
        if (p)
            goto out;
    } __except (1) {
        goto out;
    }
out:
    return r;
}

int leave(int p) {
    if (p)
        __leave;
    __try {
        work(p);
    } __except (1) {
        __leave;
    }
    __try {
        __try {
            work(1);
        } __except (1) {
            __leave;
        }
    } __finally {
    }
    return 0;
}

int out_of_finally(int p) {
    while (p--) {
        __try {
            work(p);
        } __finally {
            if (p == 1)
                continue;
            if (p == 2)
                break;
            if (p == 3)
                goto done;
            while (p)
                break;
        }
    }
done:
    __try {
        work(p);
    } __finally {
        if (p)
            return 1;
    }
    return 0;
}

int computed(int p) {
    static void *targets[] = {&&target, &&end};
    goto *targets[p];
    __try {
    target:
        work(p);
    } __finally {
    }
end:
    return 0;
}
