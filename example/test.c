// #include "prelude.h"
#include <stdio.h>
#include <stdlib.h>
int main(void) {
  int a[1];
  int aa=1;
  float b=aa;
  int *p;
  p=malloc(sizeof(char)*5);
  *p=1;
  free(p);
  // free(p); 
  printf("%s,%d\n", "Hellow",*(a));
  return 0;
}
