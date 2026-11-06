// Warnings of every phase under `#pragma clang diagnostic`, by source order.
// The preprocessor's, off and on again; push and pop restore a setting.
#pragma clang diagnostic ignored "-Wundef"
#if QUIET
#endif
#pragma clang diagnostic warning "-Wundef"
#if LOUD
#endif
#pragma clang diagnostic push
#pragma clang diagnostic error "-Wundef"
#if PROMOTED
#endif
#pragma clang diagnostic pop
#if RESTORED
#endif

// A macro counts where it is expanded, not where it is defined.
#pragma clang diagnostic warning "-Wint-conversion"
#define DEFINED_LOUD char *defined_loud = 1;
#pragma clang diagnostic ignored "-Wint-conversion"
DEFINED_LOUD
#define DEFINED_QUIET char *defined_quiet = 2;
#pragma clang diagnostic warning "-Wint-conversion"
DEFINED_QUIET

// `_Pragma` in a macro governs the tokens after it in the expansion.
#define PUSH_QUIET \
    _Pragma("clang diagnostic push") _Pragma("clang diagnostic ignored \"-Wint-conversion\"")
#define POP _Pragma("clang diagnostic pop")
#define QUIET_DECLARATION(declaration) PUSH_QUIET declaration POP
QUIET_DECLARATION(char *wrapped = 3;)
char *after_wrapped = 4;

// Each phase's warning reads the setting where its token came from, though
// semantic analysis runs once the whole unit is preprocessed.
#pragma clang diagnostic ignored "-Wunknown-escape-sequence"
const char quiet_escape[] = "\q";
#pragma clang diagnostic warning "-Wunknown-escape-sequence"
const char loud_escape[] = "\q";
#pragma clang diagnostic ignored "-W#warnings"
#warning quiet
#pragma clang diagnostic error "-W#warnings"
#warning promoted
#pragma clang diagnostic warning "-W#warnings"

// A header read twice has each setting in turn, and its pragmas outlast it.
#pragma clang diagnostic ignored "-Wint-conversion"
#define twice first_twice
#include "pragma-diagnostic-twice.h"
#undef twice
#pragma clang diagnostic warning "-Wint-conversion"
#define twice second_twice
#include "pragma-diagnostic-twice.h"
#undef twice
#include "pragma-diagnostic-quiet.h"
char *quiet_after_header = 5;
