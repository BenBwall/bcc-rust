// errors: 0
typedef int V __attribute__((vector_size(16))); V f(V v) { V w={v}; V lanes={1,2,3,4}; V array[2]={{v},{lanes}}; return w+array[1]; } int following;
