char *a = u"a" U"b";
int b = u8'é';
#define V(...) __VA_OPT__(##)
#embed "missing-lexpp-resource.bin"
int query = __has_include("missing.h");
int after;
