/* Comments and splices between `#`, `define` and the macro's name. */
# /*comment*/ define BAD missing_name
#/**/define/**/WORSE missing_other
%:define DIGRAPH missing_digraph
# \
define SPLICED missing_spliced
int f(void) { return BAD + WORSE + DIGRAPH + SPLICED; }
