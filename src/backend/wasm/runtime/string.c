/* <string.h>, <strings.h> and <ctype.h> for wasm32-wasip1, in the "C"
   locale. C99: §7.21, pp. 325-336; §7.4, pp. 181-187. */
#include <ctype.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

void *memcpy(void *restrict d, const void *restrict s, size_t n) {
    unsigned char *p = d;
    const unsigned char *q = s;
    while (n--)
        *p++ = *q++;
    return d;
}

void *memmove(void *d, const void *s, size_t n) {
    unsigned char *p = d;
    const unsigned char *q = s;
    if (p < q) {
        while (n--)
            *p++ = *q++;
    } else {
        while (n--)
            p[n] = q[n];
    }
    return d;
}

void *memset(void *d, int c, size_t n) {
    unsigned char *p = d;
    while (n--)
        *p++ = (unsigned char)c;
    return d;
}

int memcmp(const void *a, const void *b, size_t n) {
    const unsigned char *p = a, *q = b;
    for (; n; n--, p++, q++)
        if (*p != *q)
            return *p - *q;
    return 0;
}

void *memchr(const void *s, int c, size_t n) {
    const unsigned char *p = s;
    for (; n; n--, p++)
        if (*p == (unsigned char)c)
            return (void *)p;
    return 0;
}

size_t strlen(const char *s) {
    const char *p = s;
    while (*p)
        p++;
    return (size_t)(p - s);
}

size_t strnlen(const char *s, size_t n) {
    size_t i = 0;
    while (i < n && s[i])
        i++;
    return i;
}

int strcmp(const char *a, const char *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return (unsigned char)*a - (unsigned char)*b;
}

int strncmp(const char *a, const char *b, size_t n) {
    for (; n; n--, a++, b++) {
        if (*a != *b)
            return (unsigned char)*a - (unsigned char)*b;
        if (!*a)
            return 0;
    }
    return 0;
}

int strcoll(const char *a, const char *b) { return strcmp(a, b); }

size_t strxfrm(char *restrict d, const char *restrict s, size_t n) {
    size_t length = strlen(s);
    if (length < n)
        memcpy(d, s, length + 1);
    return length;
}

char *strcpy(char *restrict d, const char *restrict s) {
    char *p = d;
    while ((*p++ = *s++))
        ;
    return d;
}

char *strncpy(char *restrict d, const char *restrict s, size_t n) {
    size_t i = 0;
    for (; i < n && s[i]; i++)
        d[i] = s[i];
    for (; i < n; i++)
        d[i] = 0;
    return d;
}

char *strcat(char *restrict d, const char *restrict s) {
    strcpy(d + strlen(d), s);
    return d;
}

char *strncat(char *restrict d, const char *restrict s, size_t n) {
    char *p = d + strlen(d);
    while (n-- && *s)
        *p++ = *s++;
    *p = 0;
    return d;
}

char *strchr(const char *s, int c) {
    for (;; s++) {
        if (*s == (char)c)
            return (char *)s;
        if (!*s)
            return 0;
    }
}

char *strrchr(const char *s, int c) {
    const char *found = 0;
    for (;; s++) {
        if (*s == (char)c)
            found = s;
        if (!*s)
            return (char *)found;
    }
}

char *strstr(const char *haystack, const char *needle) {
    size_t n = strlen(needle);
    for (; *haystack; haystack++)
        if (!strncmp(haystack, needle, n))
            return (char *)haystack;
    return n ? 0 : (char *)haystack;
}

size_t strspn(const char *s, const char *accept) {
    size_t i = 0;
    while (s[i] && strchr(accept, s[i]))
        i++;
    return i;
}

size_t strcspn(const char *s, const char *reject) {
    size_t i = 0;
    while (s[i] && !strchr(reject, s[i]))
        i++;
    return i;
}

char *strpbrk(const char *s, const char *accept) {
    s += strcspn(s, accept);
    return *s ? (char *)s : 0;
}

char *strtok(char *restrict s, const char *restrict delimiters) {
    static char *next;
    if (!s)
        s = next;
    if (!s)
        return 0;
    s += strspn(s, delimiters);
    if (!*s) {
        next = 0;
        return 0;
    }
    char *end = s + strcspn(s, delimiters);
    if (*end) {
        *end = 0;
        next = end + 1;
    } else {
        next = 0;
    }
    return s;
}

char *strdup(const char *s) {
    size_t n = strlen(s) + 1;
    char *p = malloc(n);
    return p ? memcpy(p, s, n) : 0;
}

char *strndup(const char *s, size_t n) {
    size_t length = strnlen(s, n);
    char *p = malloc(length + 1);
    if (!p)
        return 0;
    memcpy(p, s, length);
    p[length] = 0;
    return p;
}

char *strerror(int error) {
    switch (error) {
    case 0: return "Success";
    case 2: return "Permission denied";
    case 8: return "Bad file descriptor";
    case 18: return "Numerical argument out of domain";
    case 25: return "Invalid or incomplete multibyte or wide character";
    case 28: return "Invalid argument";
    case 29: return "Input/output error";
    case 44: return "No such file or directory";
    case 48: return "Cannot allocate memory";
    case 52: return "Function not implemented";
    case 63: return "Operation not permitted";
    case 68: return "Numerical result out of range";
    default: return "Unknown error";
    }
}

int strcasecmp(const char *a, const char *b) {
    while (*a && tolower((unsigned char)*a) == tolower((unsigned char)*b)) {
        a++;
        b++;
    }
    return tolower((unsigned char)*a) - tolower((unsigned char)*b);
}

int strncasecmp(const char *a, const char *b, size_t n) {
    for (; n; n--, a++, b++) {
        int x = tolower((unsigned char)*a), y = tolower((unsigned char)*b);
        if (x != y)
            return x - y;
        if (!x)
            return 0;
    }
    return 0;
}

int isalnum(int c) { return isalpha(c) || isdigit(c); }
int isalpha(int c) { return (unsigned)((c | 32) - 'a') < 26; }
int isblank(int c) { return c == ' ' || c == '\t'; }
int iscntrl(int c) { return (unsigned)c < 32 || c == 127; }
int isdigit(int c) { return (unsigned)(c - '0') < 10; }
int isgraph(int c) { return (unsigned)(c - 33) < 94; }
int islower(int c) { return (unsigned)(c - 'a') < 26; }
int isprint(int c) { return (unsigned)(c - 32) < 95; }
int ispunct(int c) { return isgraph(c) && !isalnum(c); }
int isspace(int c) { return c == ' ' || (unsigned)(c - '\t') < 5; }
int isupper(int c) { return (unsigned)(c - 'A') < 26; }
int isxdigit(int c) { return isdigit(c) || (unsigned)((c | 32) - 'a') < 6; }
int tolower(int c) { return isupper(c) ? c | 32 : c; }
int toupper(int c) { return islower(c) ? c & 0x5f : c; }
