/* Counts the solutions of the N-queens problem by backtracking with column
 * and diagonal occupancy arrays. Recursion, small arrays, compare and branch. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define N 8
#define EXPECT 3689921444u
#else
#define N 13
#define EXPECT 2238678269u
#endif

static int column_used[N];
static int diagonal_used[2 * N];
static int anti_used[2 * N];

static unsigned place(int row) {
  if (row == N) return 1;
  unsigned found = 0;
  for (int col = 0; col < N; col++) {
    int d = row + col;
    int a = row - col + N - 1;
    if (column_used[col] || diagonal_used[d] || anti_used[a]) continue;
    column_used[col] = diagonal_used[d] = anti_used[a] = 1;
    found += place(row + 1);
    column_used[col] = diagonal_used[d] = anti_used[a] = 0;
  }
  return found;
}

int main(void) {
  unsigned solutions = place(0);
  unsigned checksum = solutions * 2654435761u + N;
  printf("nqueens: n %d solutions %u checksum %u\n", N, solutions, checksum);
  return checksum == EXPECT ? 0 : 1;
}
