#define EMPTY
#define E2 EMPTY
#define ID(x) x
#define ID2(x) ID(x)
#define CAT(a,b) a##b
#define XCAT(a,b) CAT(a,b)
#define STR(x) #x
#define XSTR(x) STR(x)
#define W(x) [x]
#define W2(x) [ x ]
#define OBJ a b
#define OBJ2 +
#define OBJ3 OBJ2 OBJ2
#define FN() f
#define TWO(a,b) a b
#define TWO2(a,b) a,b
#define TWO3(a,b) (a)(b)
#define VA(...) __VA_ARGS__
#define VA1(x,...) x,##__VA_ARGS__
#define VA2(x,...) x, ##__VA_ARGS__
#define VA3(x,...) x __VA_ARGS__
#define VO(...) __VA_OPT__(a)
#define VO2(...) x __VA_OPT__(a) y
#define VO3(...) x __VA_OPT__(a)y
#define VO4(...) x __VA_OPT__()y
#define VO5(...) x+__VA_OPT__()+y
#define VO6(...) x __VA_OPT__(__VA_ARGS__) y
#define VO7(...) x+__VA_OPT__(__VA_ARGS__)+y
#define VO8(...) #__VA_OPT__(x)y
#define VO9(...) a#__VA_OPT__(x)
#define VO10(...) a #__VA_OPT__(x) b
#define VO11(...) a __VA_OPT__(b c) d
#define LP (
#define RP )
#define L2(x) (x
#define APPLY(m, x) m(x)
#define LATE ID
#define DEFER(x) x EMPTY
#define PLUSX +x
#define MINUS -
#define DOT .
#define NUM 1
#define NAME n
#define H1(x) -x
#define H2(x) - x
#define H3(x) x-
#define H4(x) x -
#define H5(x) x-x
#define H6(x) x x
#define H7(x) -x-
#define PA(x) x##1
#define PB(x) 1##x
#define PC(x) -x##1
#define PD(x) - x##1
#define PE(x) 1##x-
#define PF(x,y) x##y
#define PG(x,y) -x##y-
#define PH(x,y) - x ## y -
#define PI2(x,y,z) x##y##z
#define PJ(x,y,z) -x##y##z-
#define SA(x) #x
#define SB(x) a#x
#define SC(x) a #x
#define SD(x) a#x b
#define SE(x) a #x"b"
#define SF(x) #x#x
#define SG(x) #x #x
#define SH(x) L#x
#define SI(x) (#x)
a1 = ID(1);
a2=ID(1);
a3 = ID(1) ;
a4 = ID( 1 );
a5 = ID(ID(1));
a6 = ID( ID( 1 ) );
a7 = ID2(1);
a8 = ID2(ID(1));
b1 = W(1);
b2 = W( 1 );
b3 = W2(1);
b4 = W(ID(1));
b5 = W(EMPTY 1);
b6 = W(1 EMPTY);
b7 = W(EMPTY);
b8 = W( EMPTY );
b9 = W(E2 1 E2);
c1 = OBJ;
c2=OBJ;
c3 = OBJ OBJ;
c4 = OBJ+OBJ;
c5 = OBJ2 OBJ2;
c6 = OBJ2OBJ2;
c7 = +OBJ2;
c8 = OBJ2+;
c9 = OBJ3;
c10 = -MINUS;
c11 = MINUS-;
c12 = MINUS MINUS;
c13 = DOT DOT DOT;
c14 = .DOT;
c15 = DOT.;
c16 = NUM NUM;
c17 = NUM+1;
c18 = 1+NUM;
c19 = NUM.;
c20 = .NUM;
c21 = NAME NAME;
c22 = xNAME;
c23 = NAME"s";
c24 = NAME's';
c25 = NAME 1;
c26 = ID(NAME)1;
c27 = ID(NAME)L"s";
c28 = ID(L)"s";
c29 = ID(1)x;
c30 = ID(1)1;
c31 = ID(1)'c';
c32 = ID(x)'c';
c33 = ID("s")"t";
c34 = ID("s")x;
c35 = ID('c')x;
d1 = FN();
d2 = FN()();
d3 = FN;
d4 = FN x;
d5 = FN ();
d6 = FN( );
d7 = ID(FN);
d8 = ID(FN)();
d9 = LATE(1);
d10 = LATE (1);
d11 = ID(LATE)(1);
d12 = ID(ID)(1);
d13 = ID(ID) (1);
d14 = APPLY(ID,1);
d15 = APPLY(ID, 1);
d16 = APPLY(W,1);
d17 = ID LP 1 RP;
d18 = ID(ID LP 1 RP);
d19 = L2(1) x;
d20 = DEFER(ID)(1);
d21 = DEFER(ID) (1);
e1 = TWO(1,2);
e2 = TWO( 1 , 2 );
e3 = TWO(,2);
e4 = TWO(1,);
e5 = TWO(,);
e6 = TWO2(1,2);
e7 = TWO2(,);
e8 = TWO3(1,2);
e9 = TWO(EMPTY,EMPTY);
e10 = TWO(+,+);
e11 = TWO2(+,+);
e12 = TWO3(+,+);
e13 = TWO(a b,c d);
f1 = VA(1,2);
f2 = VA( 1 , 2 );
f3 = VA();
f4 = VA1(1,2);
f5 = VA1(1, 2);
f6 = VA1(1);
f7 = VA1(1,);
f8 = VA2(1,2);
f9 = VA2(1, 2);
f10 = VA2(1);
f11 = VA3(1,2);
f12 = VA3(1);
g1 = VO(1);
g2 = VO();
g3 = VO2(1);
g4 = VO2();
g5 = VO3(1);
g6 = VO3();
g7 = VO4(1);
g8 = VO4();
g9 = VO5(1);
g10 = VO5();
g11 = VO6(1);
g12 = VO6();
g13 = VO7(1);
g14 = VO7();
g15 = VO8(1);
g16 = VO8();
g17 = VO9(1);
g18 = VO9();
g19 = VO10(1);
g20 = VO10();
g21 = VO11(1);
g22 = VO11();
g23 = ID(VO(1));
g24 = W(VO(1));
g25 = W(VO());
h1 = H1(a);
h2 = H1(-);
h3 = H1( - );
h4 = H2(-);
h5 = H3(-);
h6 = H4(-);
h7 = H5(-);
h8 = H6(-);
h9 = H7(-);
h10 = -H1(-);
h11 = -H2(-);
h12 = H3(-)-;
h13 = H4(-)-;
h14 = H7(>);
h15 = H5(>);
i1 = PA(x);
i2 = PB(2);
i3 = PC(x);
i4 = PD(x);
i5 = PE(2);
i6 = PF(a,b);
i7 = PF(,b);
i8 = PF(a,);
i9 = PF(,);
i10 = PG(a,b);
i11 = PG(,b);
i12 = PG(a,);
i13 = PG(,);
i14 = PH(a,b);
i15 = PH(,b);
i16 = PH(a,);
i17 = PH(,);
i18 = PF(a b,c d);
i19 = PG(a b,c d);
i20 = PF(a -,- d);
i21 = PF(a 1,2 +)+;
i22 = PI2(a,b,c);
i23 = PJ(a,b,c);
i24 = PJ(,,);
i25 = PJ(a b,c d,e f);
i26 = CAT(1,2)+;
i27 = +CAT(1,2);
i28 = CAT(+,+)+;
i29 = XCAT(ID(1),ID(2));
i30 = XCAT(NUM,NUM);
i31 = XCAT(1,2)x;
i32 = CAT(ID,)(1);
i33 = CAT(I,D)(1);
i34 = CAT(I,D) (1);
i35 = -CAT(MI,NUS)-;
j1 = SA(x);
j2 = SA( x y );
j3 = SB(x);
j4 = SC(x);
j5 = SD(x);
j6 = SE(x);
j7 = SF(x);
j8 = SG(x);
j9 = SH(x);
j10 = SI(x);
j11 = XSTR(ID(1));
j12 = XSTR(ID( 1 ) + ID(2));
j13 = XSTR(ID(1)ID(2));
j14 = XSTR(EMPTY x);
j15 = XSTR(x EMPTY);
j16 = XSTR(-ID(-));
j17 = XSTR(W(1));
k1 = __LINE__;
k2=__LINE__;
k3 = __LINE__+1;
k4 = 1+__LINE__;
k5 = __LINE__ __LINE__;
k6 = __LINE__.;
k7 = x__LINE__;
k8 = ID(__LINE__);
k9 = W(__LINE__);
k11 = ID(__COUNTER__)__COUNTER__;
k12 = -__COUNTER__-;
k13 = __has_include("nonexistent.h")+;
k14 = +__has_include("nonexistent.h");
k15 = __has_include("nonexistent.h") x;
m1 = EMPTY x;
m2=EMPTY x;
m3 = x EMPTY;
m4 = x EMPTY y;
m5 = x EMPTY EMPTY y;
m6 = xEMPTY y;
m7 = x EMPTY+y;
m8 = +EMPTY+;
m9 = + EMPTY +;
m10 = +EMPTY +;
m11 = + EMPTY+;
m12 = -E2-;
m13 = ID()ID();
m14 = -ID()-;
m15 = - ID() -;
m16 = -ID( )-;
m17 = x ID() y;
n1 = :ID(:);
n2 = <ID(<);
n3 = <ID(=);
n4 = <ID(:);
n5 = <ID(%);
n6 = %ID(:);
n7 = %ID(>);
n8 = %ID(=);
n9 = /ID(/);
n10 = /ID(*);
n11 = ID(/)/;
n12 = ID(/)*;
n13 = &ID(&);
n14 = |ID(|);
n15 = ^ID(=);
n16 = !ID(=);
n17 = =ID(=);
n18 = *ID(=);
n19 = >ID(>);
n20 = >ID(=);
n21 = >>ID(=);
n22 = <<ID(=);
n23 = >>ID(>);
n24 = ->ID(*);
n25 = ID(-)>;
n26 = .ID(.);
n27 = .ID(%);
n28 = .ID(1);
n29 = ID(.)1;
n30 = ID(.).;
n31 = ...ID(.);
n32 = #ID(#);
n33 = #ID(%);
n34 = %:ID(%:);
n35 = ID(%:)#;
n36 = <:ID(:>);
n37 = <%ID(%>);
n38 = ID(<:)x;
n39 = ID(:>)>;
n40 = ID(<%)%;
n41 = ID(%>)>;
n42 = \ID(x);
n43 = ID(\)x;
n44 = ID(\)1;
n45 = @ID(x);
n46 = ID(@)x;
n47 = $ID(x);
n48 = ID($)x;
n49 = ID(x)$;
n50 = ID(1)$;
n51 = ID(`)x;
o1 = ID(1)ID(2)ID(3);
o2 = ID(a)ID(b)ID(c);
o3 = ID(+)ID(+)ID(+);
o4 = ID(+) ID(+);
o5 = ID(ID(ID(ID(+))))ID(ID(+));
o6 = ID(+ID(+));
o7 = ID(ID(+)+);
o8 = ID(ID(+)ID(+));
o9 = ID(ID(a)ID(b));
o10 = ID(ID(a) ID(b));
o11 = W(ID(a)ID(b));
o12 = W(ID(a) ID(b));
o13 = W(W(a));
o14 = W(W( a ));
o15 = W( W(a) );
o16 = TWO(ID(1),ID(2));
o17 = TWO2(ID(1),ID(2));
o18 = TWO3(ID(1),ID(2));
