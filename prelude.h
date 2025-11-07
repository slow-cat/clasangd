#ifndef PRELUDE_H
#define PRELUDE_H
inline int prelude(int x) { return 3 * x; }

#ifdef TESTTESTTEST
#undef TESTTESTTEST
// #include "prelude.h"
#include <stdio.h>
int main(void) {
  printf("%d\n", prelude(1));
  return 0;
}
#endif // TESTTESTTEST

#endif // PRELUDE_H
example/test.c
