#include <record-macros.h>
struct S { int a, b; };
typedef struct S T;
/* Silent: `input_location` stays at the tag a system macro spells. */
struct TAG tag = {1};
WHOLE_TAG whole_tag = {1};
WHOLE_DECLARATION
void f(void) { struct TAG local = {1}; (void)local; }
struct TAG first = {1, 2}, second = {1};
struct S before_user; struct TAG after_user = {1};
/* Warned about: a new line moves it. */
struct TAG next_line =
    {1};
struct TAG
    name_on_next_line = {1};
struct TAG closing_on_next_line = {1
};
WHOLE_TAG whole_tag_next_line =
    {1};
/* Warned about: a later user tag moves it back. */
struct TAG before_tag; struct S after_tag = {1};
/* Warned about: nothing else about the spelling matters. */
KEYWORD S keyword = {1};
ALIAS alias = {1};
struct S braces = BRACED;
struct S close = {1 CLOSE;
ALIAS_DECLARATION
int following_valid;
