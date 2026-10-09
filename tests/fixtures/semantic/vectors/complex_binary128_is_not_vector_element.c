// errors: 1
typedef _Complex __float128 Bad __attribute__((vector_size(32))); typedef __float128 Good __attribute__((vector_size(32))); Good valid; int following;
