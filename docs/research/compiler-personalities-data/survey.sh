# usage: survey.sh LABEL CMD...
S=<scratch>
L=$1; shift
O=$S/out/$L; mkdir -p $O
cd $S/probe
{ echo "\$ $* --version"; timeout 30 "$@" --version 2>&1; echo "status $?"; } > $O/version.txt
{ echo "\$ $* -v -c empty.c -o /dev/null"; timeout 30 "$@" -v -c empty.c -o $O/empty.o 2>&1; echo "status $?"; } > $O/v.txt
{ timeout 30 "$@" -dM -E empty.c 2>&1; echo "status $?"; } > $O/dM.txt
{ for f in undeclared semicolon member args redecl lvalue header brk gotos warn; do echo "== $f.c"; timeout 30 "$@" -c $f.c -o $O/x.o 2>&1; echo "status $?"; done
  echo "== missing.c"; timeout 30 "$@" -c missing.c -o $O/x.o 2>&1; echo "status $?"
  echo "== bogus option"; timeout 30 "$@" -fbogus-option -c empty.c -o $O/x.o 2>&1; echo "status $?"
  echo "== ok.c (link+run)"; timeout 60 "$@" ok.c -o $O/ok 2>&1 && $O/ok; echo "status $?"; } > $O/diag.txt
