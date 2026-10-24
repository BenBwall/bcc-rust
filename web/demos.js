// These are C source programs, not execution traces. Rust produces every state.
export const demos = {
  tour: {
    name: "objects.c",
    source: `int bump(int *value) {
    int before = *value;
    *value = before + 1;
    return *value;
}

int main(void) {
    int total = 0;
    int values[3] = {2, 4, 6};
    int *cursor = &values[0];

    for (int i = 0; i < 3; i = i + 1) {
        int next = bump(cursor);
        total = total + next;
        cursor = cursor + 1;
    }

    return total;
}
`,
  },
  indeterminate: {
    name: "indeterminate.c",
    source: `int main(void) {
    int uninitialized;
    int result = uninitialized + 1;
    return result;
}
`,
  },
  bounds: {
    name: "bounds.c",
    source: `int main(void) {
    int values[3] = {2, 4, 6};
    int *end = values + 3;
    return *end;
}
`,
  },
};
