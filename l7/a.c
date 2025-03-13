#include <stdio.h>
#include <stdlib.h>

void one_step_in() {
    void* x = malloc(sizeof(double));
    free(x);
}

int main(int argc, char** argv){ 
    one_step_in();
    int* p = malloc(sizeof(int));
    *p = 100;
    printf("hello world %d", *p);
    free(p);
    return 0;
}
