// errors: 2
void f(_Bool a,_Bool b,int x,float y){__builtin_elementwise_min(a,b); __builtin_elementwise_max(a,b); __builtin_elementwise_min(x,x); __builtin_elementwise_max(y,y); __builtin_nondeterministic_value(a); } int following;
