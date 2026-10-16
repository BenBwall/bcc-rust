#!/usr/bin/env python3
"""A tiny m4 subset, enough to build Csmith's generated runtime headers.

Csmith's `runtime/*.m4` files use only `define`, `ifelse`, `dnl`, `#` comments,
backtick quoting and `$N` macro parameters. Windows hosts usually have no m4,
so `scripts/random_program_conformance.py build-generators` points CMake's `M4`
at a wrapper around this module. Supported: those builtins, `$0`..`$9`, `$#`,
`$*` and `$@`. Anything else is passed through, which is what GNU m4 does for
unknown words. Reads stdin, writes stdout, like `m4 < in > out`.
"""

import re
import sys

WORD = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


class MiniM4:
    def __init__(self):
        self.macros = {}

    def run(self, text):
        out = []
        index = 0
        while index < len(text):
            char = text[index]
            if char == "`":
                inner, index = self.quoted(text, index)
                out.append(inner)
            elif char == "#":
                # A comment is copied verbatim, through its newline.
                end = text.find("\n", index)
                end = len(text) if end < 0 else end + 1
                out.append(text[index:end])
                index = end
            elif match := WORD.match(text, index):
                word = match[0]
                index = match.end()
                if word == "dnl":
                    end = text.find("\n", index)
                    index = len(text) if end < 0 else end + 1
                elif word in self.macros or word in ("define", "ifelse"):
                    arguments = []
                    if index < len(text) and text[index] == "(":
                        arguments, index = self.arguments(text, index)
                    replacement = self.call(word, arguments)
                    # Rescan the replacement together with the rest of the input.
                    text = replacement + text[index:]
                    index = 0
                else:
                    out.append(word)
            else:
                out.append(char)
                index += 1
        return "".join(out)

    @staticmethod
    def quoted(text, start):
        depth = 0
        index = start
        while index < len(text):
            if text[index] == "`":
                depth += 1
            elif text[index] == "'":
                depth -= 1
                if depth == 0:
                    return text[start + 1:index], index + 1
            index += 1
        raise ValueError("unterminated quote")

    def arguments(self, text, start):
        """Split a parenthesized argument list; return expanded args and the index after `)`."""
        raw, current, depth, index = [], [], 0, start + 1
        while index < len(text):
            char = text[index]
            if char == "`":
                _, end = self.quoted(text, index)
                current.append(text[index:end])
                index = end
                continue
            if char == "(":
                depth += 1
            elif char == ")":
                if depth == 0:
                    raw.append("".join(current))
                    return [self.run(item.lstrip()) for item in raw], index + 1
                depth -= 1
            elif char == "," and depth == 0:
                raw.append("".join(current))
                current = []
                index += 1
                continue
            current.append(char)
            index += 1
        raise ValueError("unterminated macro call")

    def call(self, name, arguments):
        if name == "define":
            if arguments:
                self.macros[arguments[0]] = arguments[1] if len(arguments) > 1 else ""
            return ""
        if name == "ifelse":
            items = list(arguments)
            while True:
                if len(items) <= 1:
                    return items[0] if items else ""
                if len(items) == 2:
                    return ""
                if items[0] == items[1]:
                    return items[2]
                if len(items) == 3:
                    return ""
                if len(items) == 4:
                    return items[3]
                items = items[3:]
        body = self.macros[name]

        def parameter(match):
            token = match[1]
            if token == "#":
                return str(len(arguments))
            if token == "*":
                return ",".join(arguments)
            if token == "@":
                return ",".join("`" + item + "'" for item in arguments)
            number = int(token)
            if number == 0:
                return name
            return arguments[number - 1] if number <= len(arguments) else ""

        return re.sub(r"\$([0-9#*@])", parameter, body)


def main():
    sys.stdout.write(MiniM4().run(sys.stdin.read()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
