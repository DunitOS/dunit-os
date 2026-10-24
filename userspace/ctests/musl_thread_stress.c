#include <pthread.h>
#include <sched.h>
#include <errno.h>
#include <stdlib.h>
#include <stdio.h>
#include <stdint.h>
static _Thread_local int local_value=7;
static unsigned completed;
static void *worker(void *arg) {
    intptr_t id=(intptr_t)arg;
    if(local_value!=7)return (void *)1;
    local_value=(int)id*3+7;
    errno=(int)id+100;
    unsigned char *bytes=malloc(48);
    if(!bytes)return (void *)2;
    for(int i=0;i<48;i++)bytes[i]=(unsigned char)(i+id);
    for(int i=0;i<10;i++)sched_yield();
    int valid=local_value==(int)id*3+7 && errno==(int)id+100;
    for(int i=0;i<48;i++)if(bytes[i]!=(unsigned char)(i+id))valid=0;
    free(bytes);
    if(!valid)return (void *)3;
    __atomic_fetch_add(&completed,1,__ATOMIC_RELAXED);
    return 0;
}
int main(void) {
    for(intptr_t id=0;id<1000;id++){
        pthread_t thread;void *result=(void *)1;
        int rc=pthread_create(&thread,0,worker,(void *)id);
        int joined=rc?0:pthread_join(thread,&result);
        if(rc || joined || result){printf("[THREAD-STRESS] FAIL cycle=%ld create=%d join=%d result=%ld\n",(long)id,rc,joined,(long)result);return 1;}
    }
    if(completed!=1000 || local_value!=7){puts("[THREAD-STRESS] FAIL completion/TLS");return 1;}
    puts("[THREAD-STRESS] cycles=1000 TLS/errno/allocation OK");
    pthread_attr_t attr;
    if(pthread_attr_init(&attr) || pthread_attr_setdetachstate(&attr,PTHREAD_CREATE_DETACHED))return 1;
    for(intptr_t id=1000;id<2000;id++){
        pthread_t thread;
        int rc=pthread_create(&thread,&attr,worker,(void *)id);
        if(rc){printf("[THREAD-STRESS] FAIL detached cycle=%ld create=%d\n",(long)id,rc);return 1;}
        unsigned spins=0;
        while(__atomic_load_n(&completed,__ATOMIC_ACQUIRE)!=(unsigned)id+1){
            if(++spins==1000000){puts("[THREAD-STRESS] FAIL detached completion");return 1;}
            sched_yield();
        }
    }
    pthread_attr_destroy(&attr);
    /* Let the last detached worker finish its exit after publishing completion. */
    for(int i=0;i<10;i++)sched_yield();
    puts("[THREAD-STRESS] detached=1000 auto-reap OK");return 0;
}
