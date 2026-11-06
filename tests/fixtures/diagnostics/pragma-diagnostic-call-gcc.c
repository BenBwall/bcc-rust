// A call's warning comes from after lowering's reachability, and is reported
// where the call is.
__attribute__((warning("careful"))) void careful(void);
void call_loud(void) { careful(); }
#pragma GCC diagnostic ignored "-Wattribute-warning"
void call_quiet(void) { careful(); }
#pragma GCC diagnostic error "-Wattribute-warning"
void call_promoted(void) { careful(); }
