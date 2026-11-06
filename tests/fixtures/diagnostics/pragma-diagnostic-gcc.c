// Warnings of every phase under `#pragma GCC diagnostic`, by source order.
// The preprocessor's, off and on again; push and pop restore a setting.
#pragma GCC diagnostic ignored "-Wundef"
#if QUIET
#endif
#pragma GCC diagnostic warning "-Wundef"
#if LOUD
#endif
#pragma GCC diagnostic push
#pragma GCC diagnostic error "-Wundef"
#if PROMOTED
#endif
#pragma GCC diagnostic pop
#if RESTORED
#endif

// A macro counts where it is expanded, not where it is defined.
#pragma GCC diagnostic warning "-Wint-conversion"
#define DEFINED_LOUD char *defined_loud = 1;
#pragma GCC diagnostic ignored "-Wint-conversion"
DEFINED_LOUD
#define DEFINED_QUIET char *defined_quiet = 2;
#pragma GCC diagnostic warning "-Wint-conversion"
DEFINED_QUIET

// `_Pragma` in a macro governs the tokens after it in the expansion.
#define PUSH_QUIET \
    _Pragma("GCC diagnostic push") _Pragma("GCC diagnostic ignored \"-Wint-conversion\"")
#define POP _Pragma("GCC diagnostic pop")
#define QUIET_DECLARATION(declaration) PUSH_QUIET declaration POP
QUIET_DECLARATION(char *wrapped = 3;)
char *after_wrapped = 4;

// Each phase's warning reads the setting where its token came from, though
// semantic analysis runs once the whole unit is preprocessed.
#pragma GCC diagnostic ignored "-Wunknown-escape-sequence"
const char quiet_escape[] = "\q";
#pragma GCC diagnostic warning "-Wunknown-escape-sequence"
const char loud_escape[] = "\q";
#pragma GCC diagnostic ignored "-Wcpp"
#warning quiet
#pragma GCC diagnostic error "-Wcpp"
#warning promoted
#pragma GCC diagnostic warning "-Wcpp"

// A header read twice has each setting in turn, and its pragmas outlast it.
#pragma GCC diagnostic ignored "-Wint-conversion"
#define twice first_twice
#include "pragma-diagnostic-twice.h"
#undef twice
#pragma GCC diagnostic warning "-Wint-conversion"
#define twice second_twice
#include "pragma-diagnostic-twice.h"
#undef twice
#include "pragma-diagnostic-quiet.h"
char *quiet_after_header = 5;
