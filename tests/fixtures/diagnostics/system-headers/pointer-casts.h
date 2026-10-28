#define TO_POINTER(x) ((void *)(x))
#define TO_INT(p) ((int)(p))
static inline void *header_cast(int i) { return (void *)i; }
