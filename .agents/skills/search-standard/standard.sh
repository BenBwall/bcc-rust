#!/bin/sh
# Look things up in the generated HTML editions of the C standard in standards/.
#
#   standard.sh <edition> para  <id>      text of a clause heading, paragraph, or footnote
#   standard.sh <edition> pages <id>      PDF and printed pages an id starts or continues on
#   standard.sh <edition> find  <phrase>  every occurrence, with its clause or paragraph and page
#
# <edition> is c89, c99, c11, c17, c23, or all (every edition, each output line
# prefixed with its edition). Ids are `6.7.2` (clause), `6.7.2p2` (numbered
# paragraph), `A.1` (annex clause), `foreword-p1` (unnumbered front matter), and
# `fn104` (footnote). Matching is on text with tags removed and entities
# decoded, so phrases may span inline code, italics, and links.
set -eu

usage() { sed -n '4,6p' "$0" | sed 's/^# //' >&2; exit 2; }
[ $# -eq 3 ] || usage
edition=$1 command=$2 argument=$3
case $command in para | pages | find) ;; *) usage ;; esac

# Shared awk: track the current page and the nearest clause or paragraph, and
# expose the line as plain text.
prelude='
  /<section class="page"/ {
    match($0, /data-page="[0-9]+"/); pdf = substr($0, RSTART + 11, RLENGTH - 12)
    printed = "?"
    if (match($0, /data-printed="[^"]*"/)) printed = substr($0, RSTART + 14, RLENGTH - 15)
  }
  match($0, /<h[2-6] id="[^"]+"|class="para[^"]*" (id|data-of)="[^"]+"|class="fn" id="[^"]+"/) {
    where = substr($0, RSTART, RLENGTH); sub(/.*="/, "", where); sub(/"$/, "", where)
  }
  {
    text = $0
    # N2310 marks C2x changes against C17; C17 reads without its insertions.
    while (drop_ins && match(text, /<ins>/)) {
      rest = substr(text, RSTART); end = index(rest, "</ins>")
      if (!end) break
      text = substr(text, 1, RSTART - 1) substr(rest, end + 6)
    }
    gsub(/<[^>]*>/, "", text)
    gsub(/&lt;/, "<", text); gsub(/&gt;/, ">", text); gsub(/&quot;/, "\"", text); gsub(/&amp;/, "\\&", text)
  }
  function starts(id) { return index($0, "id=\"" id "\"") || index($0, "data-of=\"" id "\"") }
'

# Runs the command on one edition's file; exits 1 when nothing matched.
search() {
  drop_ins=0
  case $1 in *c17-*) drop_ins=1 ;; esac
  case $command in
    para)
      # From the id's element to the next paragraph, heading, or page
      # furniture; a paragraph continued on later pages is followed there.
      awk -v drop_ins="$drop_ins" -v id="$argument" "$prelude"'
        starts(id) { on = 1; found = 1; if (text ~ /[^ \t]/) print text; next }
        /^<div class="para|^<h[2-6]|^<div class="(notes|run|sep)|^<p class="fn"|^<\/section>/ { on = 0 }
        on && text ~ /[^ \t]/ { print text }
        END { exit !found }
      ' "$1"
      ;;
    pages)
      awk -v drop_ins="$drop_ins" -v id="$argument" "$prelude"'
        starts(id) { print id ": PDF p. " pdf ", printed p. " printed " (line " NR ")"; found = 1 }
        END { exit !found }
      ' "$1"
      ;;
    find)
      awk -v drop_ins="$drop_ins" -v q="$argument" "$prelude"'
        pdf != "" && index(tolower(text), tolower(q)) { print where ", PDF p. " pdf ", printed p. " printed " (line " NR ")"; found = 1 }
        END { exit !found }
      ' "$1"
      ;;
  esac
}

file_for() {
  for f in standards/"$1"-*.html; do
    [ -f "$f" ] && { echo "$f"; return 0; }
  done
  echo "standard.sh: no HTML edition for $1 in standards/; run from the repository root" >&2
  exit 2
}

if [ "$edition" = all ]; then
  status=1
  for e in c89 c99 c11 c17 c23; do
    file=$(file_for "$e")
    if out=$(search "$file"); then
      printf '%s\n' "$out" | sed "s/^/$e: /"
      status=0
    fi
  done
  [ $status -eq 0 ] || echo "standard.sh: nothing for $command $argument in any edition" >&2
  exit $status
fi

case $edition in c89 | c99 | c11 | c17 | c23) ;; *) usage ;; esac
file=$(file_for "$edition")
search "$file" || { echo "standard.sh: nothing for $command $argument in $edition" >&2; exit 1; }
