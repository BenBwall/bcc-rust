enum E { A, B }; enum N { M = -1 };
enum E *pe; enum N *pn; unsigned *pu; int *pi;
void signed_enum(void) { pu = pn; }
void same_sign(void) { pi = pn; }
void unsigned_enum(void) { pi = pe; }
void int_enum_on_msvc(void) { pu = pe; }
int following_valid;
