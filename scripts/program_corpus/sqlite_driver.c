/* Runs a SQL script through the SQLite C API and prints every result row.
 *
 * Used when the shell (shell.c) cannot be built: the output format is simple
 * enough that every correct build of sqlite3.c prints the same text.
 * Usage: sqlite_driver DATABASE SCRIPT
 * Shell dot-commands (lines starting with '.') and comment lines are skipped.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "sqlite3.h"

static int print_row(void *unused, int columns, char **values, char **names)
{
    int i;
    (void)unused;
    (void)names;
    for (i = 0; i < columns; i++) {
        if (i) putchar('|');
        fputs(values[i] ? values[i] : "", stdout);
    }
    putchar('\n');
    return 0;
}

static char *read_script(const char *path)
{
    FILE *file = fopen(path, "rb");
    long size;
    char *text, *out;
    char *line;
    if (!file) return NULL;
    fseek(file, 0, SEEK_END);
    size = ftell(file);
    fseek(file, 0, SEEK_SET);
    text = malloc((size_t)size + 1);
    out = malloc((size_t)size + 1);
    if (!text || !out || fread(text, 1, (size_t)size, file) != (size_t)size) return NULL;
    text[size] = 0;
    fclose(file);
    out[0] = 0;
    for (line = strtok(text, "\n"); line; line = strtok(NULL, "\n")) {
        if (line[0] == '.' || (line[0] == '-' && line[1] == '-')) continue;
        strcat(out, line);
        strcat(out, "\n");
    }
    free(text);
    return out;
}

int main(int argc, char **argv)
{
    sqlite3 *db;
    char *script, *cursor, *statement;
    int statements = 0, errors = 0;
    if (argc != 3) return 2;
    if (sqlite3_open(argv[1], &db) != SQLITE_OK) { puts("cannot open the database"); return 1; }
    script = read_script(argv[2]);
    if (!script) { puts("cannot read the script"); return 1; }
    printf("sqlite %s\n", sqlite3_libversion());
    /* Split the script into complete statements. */
    statement = malloc(strlen(script) + 2);
    statement[0] = 0;
    for (cursor = script; *cursor;) {
        char *end = strchr(cursor, '\n');
        size_t length = end ? (size_t)(end - cursor) + 1 : strlen(cursor);
        strncat(statement, cursor, length);
        cursor += length;
        if (sqlite3_complete(statement)) {
            char *message = NULL;
            statements++;
            if (sqlite3_exec(db, statement, print_row, NULL, &message) != SQLITE_OK) {
                errors++;
                printf("error: %s\n", message);
                sqlite3_free(message);
            }
            statement[0] = 0;
        }
    }
    printf("statements %d errors %d changes %d\n", statements, errors, sqlite3_total_changes(db));
    sqlite3_close(db);
    free(statement);
    free(script);
    return 0;
}
