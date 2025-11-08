#!/usr/bin/sh

file="$(realpath "$1")"
input="$(cat)"
# STDIN=$(grep -Po 'STDIN="\K[^"]*' "$file")
# FILEIN=$(grep -Po 'FILEIN="\K[^"]*' "$file")
STDIN=$(rg -m1 -No 'STDIN="([^"]*)"' "$file" -r '$1')
FILEIN=$(rg -m1 -No 'FILEIN="([^"]*)"' "$file" -r '$1')
if [ -n "$2" ] && [ "${#2}" -gt 1 ]; then
	DFLAGS="-D$2"
else
	DFLAGS="-DTESTTESTTEST"
fi

BIN="/tmp/c_test"
COMMON="-O0 -g -fno-omit-frame-pointer -Wall -Wextra -Wimplicit -Wconversion -fsanitize=address,undefined ${DFLAGS}"
CXXFLAGS="-std=c++17 -x c++ ${COMMON}"
CCFLAGS="-std=c17 -x c ${COMMON}"
BULDLOG="/tmp/clasangd_build.log"
RUNLOG="/tmp/clasangd_run.log"
run_with_input() {
	if [ -n "$input" ]; then
		printf "$input"|"$@"
	elif [ -n "$STDIN" ]; then
		printf '%b' "$STDIN" | "$@"
	elif [ -n "$FILEIN" ]; then
		 "$@" < "$FILEIN"
	else
		"$@"
	fi
}

compile_run() {
	if $1 $2 "$file" -o $BIN 2>${BULDLOG}&& chmod +x $BIN; then
	  cat "$BULDLOG" >&2
		run_with_input $BIN 2>"${RUNLOG}"
		cat "$RUNLOG" >&2
	fi
}
case "$file" in
*.py) run_with_input python "$file" ;;
*.tex) lualatex -i "$file" ;;
*.rs)  run_with_input cargo test $(basename "$file" .rs) --nocapture  ;;
Makefile) make ;;
*.hpp) compile_run "${CXX:-clang++}" "$CXXFLAGS" ;;
*.h) compile_run "${CC:-clang}" "$CCFLAGS" ;;
*.cpp) compile_run "${CXX:-clang++}" "$CXXFLAGS" ;;
*.c) compile_run "${CC:-clang}" "$CCFLAGS" ;;
*.ly)
	pkill -f "fluidsynth -a pipewire -i -g 2.0 "$(basename "$file" .ly).midi"" >/dev/null 2>&1
	lilypond "$file" >/dev/null 2>&1 && fluidsynth -a pipewire -i -g 2.0 "$(basename "$file" .ly).midi" >/dev/null 2>&1
	;;
*) echo "????" ;;
esac
