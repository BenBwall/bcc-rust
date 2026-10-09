#define VALUE 2j
#if __has_c_attribute(maybe_unused)
__pragma(STDC FP_CONTRACT ON)
[[maybe_unused]] __declspec(dllexport) double _Complex value = VALUE;
#endif
int following;
