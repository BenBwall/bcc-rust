// errors: 10
typedef int I __attribute__((vector_size(16))); typedef float F __attribute__((vector_size(16))); typedef short S __attribute__((vector_size(16))); void f(I a,F b,S c){a%b; a&b; b%a; b&a; a<<c; a>>c; a%=b; a&=b; a<<=c; a>>=c; a%a; a&a; a<<a; c<<c;} int following;
