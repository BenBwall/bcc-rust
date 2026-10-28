/* A first-fit allocator over the heap from __heap_base, growing memory a
   page run at a time. Every block has an 8-byte header holding its size
   (a multiple of 16, including the header) and whether it is free, and
   free blocks are merged with free neighbours that follow them.
   C99: §7.20.3, pp. 313-314. */
#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include "wasi.h"

#define HEADER 16u
#define FREE 1u

static unsigned char *heap_start, *heap_end;

static size_t *header(void *p) { return (size_t *)((unsigned char *)p - HEADER); }

/* An allocation that cannot be satisfied: null, with errno ENOMEM. */
static void *no_memory(void) {
    errno = ENOMEM;
    return 0;
}

static int grow(size_t bytes) {
    if (bytes > (size_t)-1 - 65535u)
        return 0;
    size_t pages = (bytes + 65535u) / 65536u;
    /* The heap never takes memory's last page: with all 65536 pages, its
       end, 2^32, would wrap to 0 in a pointer. */
    if (pages >= 65536u - __bcc_wasm_memory_size())
        return 0;
    if (__bcc_wasm_memory_grow(pages) == (size_t)-1)
        return 0;
    size_t end = __bcc_wasm_memory_size() * 65536u;
    /* The new run is one free block, merged with a free last block. */
    size_t *block = (size_t *)heap_end;
    block[0] = (end - (size_t)heap_end) | FREE;
    heap_end = (unsigned char *)end;
    return 1;
}

static void init(void) {
    heap_start = (unsigned char *)(((uintptr_t)&__heap_base + 15u) & ~(uintptr_t)15u);
    size_t end = __bcc_wasm_memory_size() * 65536u;
    heap_end = (unsigned char *)end;
    if (heap_end - heap_start >= 64) {
        ((size_t *)heap_start)[0] = (size_t)(heap_end - heap_start) | FREE;
    } else {
        heap_end = heap_start;
    }
}

/* Merges the free blocks that follow a free block. */
static void merge(size_t *block) {
    for (;;) {
        size_t size = block[0] & ~FREE;
        size_t *next = (size_t *)((unsigned char *)block + size);
        if ((unsigned char *)next >= heap_end || !(next[0] & FREE))
            return;
        block[0] = (size + (next[0] & ~FREE)) | FREE;
    }
}

void *malloc(size_t size) {
    if (!heap_start)
        init();
    /* Sizes this large could never be satisfied, and the header, the
       rounding and the growth step below must not wrap. */
    if (size > (size_t)-1 / 2)
        return no_memory();
    size_t need = (size + HEADER + 15u) & ~(size_t)15u;
    for (int attempt = 0; attempt < 2; attempt++) {
        size_t *last_free = 0;
        for (unsigned char *at = heap_start; at < heap_end;) {
            size_t *block = (size_t *)at;
            if (block[0] & FREE) {
                merge(block);
                size_t have = block[0] & ~FREE;
                if (have >= need) {
                    if (have - need >= 32) {
                        size_t *rest = (size_t *)(at + need);
                        rest[0] = (have - need) | FREE;
                        block[0] = need;
                    } else {
                        block[0] = have;
                    }
                    return at + HEADER;
                }
                last_free = block;
            }
            at += block[0] & ~FREE;
        }
        size_t extra = need;
        if (last_free && (unsigned char *)last_free + (last_free[0] & ~FREE) == heap_end)
            extra = need - (last_free[0] & ~FREE);
        /* A page of slack saves growing again soon, unless memory cannot
           hold it. */
        if (!grow(extra + 65536u) && !grow(extra))
            return no_memory();
        if (last_free)
            merge(last_free);
    }
    return no_memory();
}

void free(void *p) {
    if (!p)
        return;
    size_t *block = header(p);
    block[0] |= FREE;
    merge(block);
}

void *calloc(size_t count, size_t size) {
    if (size && count > (size_t)-1 / size)
        return no_memory();
    void *p = malloc(count * size);
    if (p)
        memset(p, 0, count * size);
    return p;
}

void *realloc(void *p, size_t size) {
    if (!p)
        return malloc(size);
    if (size == 0) {
        free(p);
        return 0;
    }
    size_t have = (header(p)[0] & ~FREE) - HEADER;
    if (have >= size)
        return p;
    void *q = malloc(size);
    if (q) {
        memcpy(q, p, have);
        free(p);
    }
    return q;
}

void *aligned_alloc(size_t alignment, size_t size) {
    if (alignment == 0 || (alignment & (alignment - 1)) != 0) {
        errno = EINVAL;
        return 0;
    }
    if (alignment <= 16)
        return malloc(size);
    if (size > (size_t)-1 - alignment - 32)
        return no_memory();
    unsigned char *raw = malloc(size + alignment + 32);
    if (!raw || ((uintptr_t)raw & (alignment - 1)) == 0)
        return raw;
    /* The bytes before the aligned address become a free block of their
       own, at least 32 bytes long. */
    size_t *block = header(raw);
    size_t total = block[0] & ~FREE;
    uintptr_t aligned = ((uintptr_t)raw + 32 + alignment - 1) & ~(uintptr_t)(alignment - 1);
    size_t gap = aligned - (uintptr_t)raw;
    block[0] = gap | FREE;
    header((void *)aligned)[0] = total - gap;
    return (void *)aligned;
}
