#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <spawn.h>
#include <pthread.h>
#include <sys/wait.h>
static pthread_barrier_t barrier;
static int turns[3];
static void *worker(void *arg) {
    int id=(int)(long)arg;
    for (int i=0;i<100;i++) {
        turns[id]++;
        int r=pthread_barrier_wait(&barrier);
        if(r && r!=PTHREAD_BARRIER_SERIAL_THREAD)return (void *)1;
    }
    return 0;
}
#define CHECK(v) do { if(!(v)){printf("[MUSL-PROFILE] FAIL line=%d errno=%d\n",__LINE__,errno);return 1;} }while(0)
int main(void){
    CHECK(system(0)==0);errno=0;CHECK(system("ignored")==-1 && errno==ENOSYS);
    errno=0;CHECK(popen("ignored","r")==0 && errno==ENOSYS);
    const char *path="/tmp/profile-exclusive";
    unlink(path);
    int fd=open(path,O_WRONLY|O_CREAT|O_EXCL,0600);CHECK(fd>=0);CHECK(close(fd)==0);
    errno=0;CHECK(open(path,O_WRONLY|O_CREAT|O_EXCL,0600)==-1 && errno==EEXIST);
    fd=open(path,O_RDONLY|O_CLOEXEC);CHECK(fd>=0);CHECK(fcntl(fd,F_GETFD)==FD_CLOEXEC);CHECK(close(fd)==0);
    int p[2]={-1,-1};CHECK(pipe2(p,O_CLOEXEC)==0);CHECK(fcntl(p[0],F_GETFD)==FD_CLOEXEC);CHECK(close(p[0])==0);CHECK(close(p[1])==0);
    CHECK(pipe(p)==0);CHECK(write(p[1],"ok",2)==2);char b[2];CHECK(read(p[0],b,2)==2);CHECK(close(p[1])==0);CHECK(read(p[0],b,2)==0);CHECK(close(p[0])==0);
    posix_spawn_file_actions_t fa;CHECK(!posix_spawn_file_actions_init(&fa));CHECK(!posix_spawn_file_actions_addopen(&fa,1,path,O_WRONLY,0600));
    pid_t child=123456;char *args[]={"musl_child",0};
    CHECK(posix_spawn(&child,"/app/musl_child",&fa,0,args,0)==0);int first_status;CHECK(waitpid(child,&first_status,0)==child);CHECK(WIFEXITED(first_status) && WEXITSTATUS(first_status)==7);child=123456;CHECK(!posix_spawn_file_actions_destroy(&fa));
    posix_spawnattr_t attr;CHECK(!posix_spawnattr_init(&attr));CHECK(!posix_spawnattr_setflags(&attr,POSIX_SPAWN_SETPGROUP));
    CHECK(posix_spawn(&child,"/app/musl_child",0,&attr,args,0)==ENOSYS);CHECK(child==123456);
    CHECK(setenv("PATH",":relative:/app",1)==0);CHECK(!posix_spawnp(&child,"musl_child",0,0,args,0));int status;CHECK(waitpid(child,&status,0)==child);CHECK(WIFEXITED(status));
    int previous=-1;CHECK(pthread_setcanceltype(PTHREAD_CANCEL_ASYNCHRONOUS,&previous)==ENOTSUP);CHECK(previous==-1);CHECK(pthread_cancel(pthread_self())==ENOTSUP);
    CHECK(!pthread_barrier_init(&barrier,0,3));pthread_t threads[3];
    for(long i=0;i<3;i++)CHECK(!pthread_create(&threads[i],0,worker,(void *)i));
    for(int i=0;i<3;i++){void *r=(void *)1;CHECK(!pthread_join(threads[i],&r));CHECK(!r && turns[i]==100);}
    CHECK(!pthread_barrier_destroy(&barrier));CHECK(unlink(path)==0);
    puts("[MUSL-PROFILE] flags, pipe, spawn refusal/PATH, cancellation refusal, barrier OK");return 0;
}
