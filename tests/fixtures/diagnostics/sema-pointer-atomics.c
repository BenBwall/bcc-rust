int *ip; unsigned *up; const int *cip; float *fp;
void sign(void) { __atomic_store_n(&ip, up, 0); }
void discard(void) { __atomic_store_n(&ip, cip, 0); }
void incompatible(void) { __atomic_store_n(&ip, fp, 0); }
void exchange(void) { (void)__atomic_exchange_n(&ip, up, 0); }
void sync(void) { (void)__sync_val_compare_and_swap(&ip, up, ip); }
void c11(_Atomic(int *) *a) { __c11_atomic_store(a, up, 0); }
int following_valid;
