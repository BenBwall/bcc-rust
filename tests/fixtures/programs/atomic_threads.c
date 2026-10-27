/* C11 atomics under contention: several threads, given as the first
   argument, each add ROUNDS times to counters that only an atomic
   read-modify-write keeps exact. The exit status is 0 when every counter
   is exact.

   - `counter`: atomic_fetch_add (lock xadd, or lock add when unused).
   - `doubled`: a weak compare-exchange loop adding 2 (lock cmpxchg).
   - `narrow`: `++` on an _Atomic unsigned short, which wraps.
   - `guarded`: a plain long behind an atomic_flag spin lock, whose acquire
     and release orders must keep the increments inside the lock.
   - `big`: a 16-byte __int128 counter, through libatomic. */
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>

#define ROUNDS 100000
#define LOCKED_EVERY 8

static atomic_long counter;
static long doubled;
static _Atomic unsigned short narrow;
static atomic_flag lock = ATOMIC_FLAG_INIT;
static long guarded;
static __int128 big;

static void *work(void *argument) {
  (void)argument;
  for (int i = 0; i < ROUNDS; i++) {
    atomic_fetch_add_explicit(&counter, 1, memory_order_relaxed);
    long seen = __atomic_load_n(&doubled, __ATOMIC_RELAXED);
    while (!__atomic_compare_exchange_n(&doubled, &seen, seen + 2, 1,
                                        __ATOMIC_ACQ_REL, __ATOMIC_RELAXED))
      ;
    narrow++;
    if (i % LOCKED_EVERY == 0) {
      while (atomic_flag_test_and_set_explicit(&lock, memory_order_acquire))
        ;
      guarded++;
      atomic_flag_clear_explicit(&lock, memory_order_release);
      __atomic_fetch_add(&big, 1, __ATOMIC_SEQ_CST);
    }
  }
  return 0;
}

int main(int argc, char **argv) {
  int threads = argc > 1 ? atoi(argv[1]) : 4;
  pthread_t workers[64];
  if (threads < 1 || threads > 64)
    return 2;
  for (int i = 0; i < threads; i++)
    if (pthread_create(&workers[i], 0, work, 0) != 0)
      return 3;
  for (int i = 0; i < threads; i++)
    pthread_join(workers[i], 0);
  long total = (long)threads * ROUNDS;
  long locked = (long)threads * ((ROUNDS + LOCKED_EVERY - 1) / LOCKED_EVERY);
  printf("%ld %ld %u %ld %lld\n", atomic_load(&counter), doubled,
         (unsigned)narrow, guarded, (long long)big);
  return !(atomic_load(&counter) == total && doubled == 2 * total &&
           narrow == (unsigned short)total && guarded == locked &&
           big == locked);
}
