/* An array initialized by a compound literal of its own array type (GNU),
 * as if by the literal's braced list. GCC accepts it for static storage,
 * also as an element or member inside braces; Clang also for automatic
 * arrays, but only as the whole initializer. A too-long string stores the
 * characters that fit. */
#include <stdio.h>
#include <wchar.h>

static void show(const char *name, const void *bytes, size_t size) {
    printf("%s:", name);
    for (size_t i = 0; i < size; i++)
        printf(" %02x", ((const unsigned char *)bytes)[i]);
    printf("\n");
}

static char exact[3] = (char[3]){"ab"};
char completed[] = (char[3]){"cd"};
char unsized_literal[3] = (char[]){"ef"};
char no_null[3] = (char[3]){"ghi"};
char too_long[3] = (char[3]){"jklm"};
char plain_too_long[3] = "nopq";
wchar_t wide[3] = (wchar_t[3]){L"rs"};
unsigned char unsigned_chars[3] = (unsigned char[3]){"tu"};
int integers[2] = (int[2]){1, -2};
const char constant[3] = (const char[3]){"vw"};
char parenthesized[3] = ((char[3]){"xy"});
int matrix[2][2] = (int[2][2]){{1, 2}, {3, 4}};
#ifndef __clang__
char qualified_source[3] = (const char[3]){"AB"};
struct member { char text[3]; int value; } member = { (char[3]){"CD"}, 5 };
struct member designated = { .text = (char[3]){"EF"}, 6 };
char rows[2][3] = { (char[3]){"GH"}, (char[3]){"IJ"} };
#endif

int main(void) {
    static char local_static[3] = (char[3]){"KL"};
    char short_string[2] = "MNO";
    show("exact", exact, sizeof exact);
    show("completed", completed, sizeof completed);
    show("unsized_literal", unsized_literal, sizeof unsized_literal);
    show("no_null", no_null, sizeof no_null);
    show("too_long", too_long, sizeof too_long);
    show("plain_too_long", plain_too_long, sizeof plain_too_long);
    show("wide", wide, sizeof wide);
    show("unsigned_chars", unsigned_chars, sizeof unsigned_chars);
    show("integers", integers, sizeof integers);
    show("constant", constant, sizeof constant);
    show("parenthesized", parenthesized, sizeof parenthesized);
    show("matrix", matrix, sizeof matrix);
    show("local_static", local_static, sizeof local_static);
    show("short_string", short_string, sizeof short_string);
#ifdef __clang__
    char automatic[3] = (char[3]){"OP"};
    char automatic_completed[] = (char[3]){"QR"};
    int automatic_integers[3] = (int[3]){7, 8};
    wchar_t automatic_wide[2] = (wchar_t[2]){L"S"};
    show("automatic", automatic, sizeof automatic);
    show("automatic_completed", automatic_completed, sizeof automatic_completed);
    show("automatic_integers", automatic_integers, sizeof automatic_integers);
    show("automatic_wide", automatic_wide, sizeof automatic_wide);
#else
    show("qualified_source", qualified_source, sizeof qualified_source);
    show("member", &member, sizeof member);
    show("designated", &designated, sizeof designated);
    show("rows", rows, sizeof rows);
#endif
    return 0;
}
