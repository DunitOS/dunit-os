#define _GNU_SOURCE
#include <unistd.h>
#include <fcntl.h>
#include <poll.h>
#include <sys/mman.h>
#include <sys/select.h>
#include <sys/wait.h>
#include <spawn.h>
#include <time.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
#define CHECK(x) do {if(!(x)){printf("[NATIVE-FD] FAIL line=%d errno=%d\n",__LINE__,errno);return 1;}}while(0)
int main(int argc,char **argv) {
    if(argc>1){
        if(!strcmp(argv[1],"keyboard")){
            const char marker[]="[SPAWN-INPUT] child ready\n";CHECK(write(1,marker,sizeof marker-1)==sizeof marker-1);
            char text[32];CHECK(fgets(text,sizeof text,stdin)!=0);CHECK(!strcmp(text,"spawn-read-ok\n"));
            puts("[SPAWN-INPUT] child read OK");return 9;
        }
        if(!strcmp(argv[1],"keyboard-parent")){
            char *args[]={"musl_native_fd","keyboard",0};pid_t child;int status;
            CHECK(posix_spawn(&child,"/app/musl_native_fd",0,0,args,0)==0);
            CHECK(waitpid(child,&status,0)==child && WIFEXITED(status) && WEXITSTATUS(status)==9);
            puts("[SPAWN-INPUT] parent wait OK");return 0;
        }
        if(!strcmp(argv[1],"child")){
            char b[5]={0};CHECK(read(0,b,4)==4);CHECK(!strcmp(b,"ping"));CHECK(write(1,"pong",4)==4);return 7;
        }
        if(!strcmp(argv[1],"delay")){struct timespec t={0,50000000};nanosleep(&t,0);CHECK(write(1,"x",1)==1);return 0;}
    }
    int p[2],q[2]; CHECK(pipe2(p,O_NONBLOCK|O_CLOEXEC)==0);
    CHECK(fcntl(p[0],F_GETFD)==FD_CLOEXEC);CHECK(fcntl(p[0],F_GETFL)&O_NONBLOCK);
    char b[8];errno=0;CHECK(read(p[0],b,1)==-1 && errno==EAGAIN);
    int alias=dup(p[0]);CHECK(alias>=0);CHECK(fcntl(alias,F_GETFD)==0);
    CHECK(fcntl(alias,F_SETFL,0)==0);CHECK(!(fcntl(p[0],F_GETFL)&O_NONBLOCK));
    CHECK(fcntl(alias,F_SETFD,FD_CLOEXEC)==0);CHECK(close(alias)==0);
    struct pollfd pollfd={p[0],POLLIN,0};CHECK(poll(&pollfd,1,0)==0);
    CHECK(write(p[1],"a",1)==1);fd_set ready;FD_ZERO(&ready);FD_SET(p[0],&ready);struct timeval no_wait={0,0};CHECK(select(p[0]+1,&ready,0,0,&no_wait)==1 && FD_ISSET(p[0],&ready));CHECK(poll(&pollfd,1,0)==1 && (pollfd.revents&POLLIN));CHECK(read(p[0],b,1)==1);
    CHECK(close(p[1])==0);CHECK(poll(&pollfd,1,0)==1 && (pollfd.revents&POLLHUP));CHECK(close(p[0])==0);
    pollfd.fd=999;CHECK(poll(&pollfd,1,0)==1 && (pollfd.revents&POLLNVAL));
    struct timespec start,end;CHECK(clock_gettime(CLOCK_MONOTONIC,&start)==0);CHECK(poll(0,0,40)==0);CHECK(clock_gettime(CLOCK_MONOTONIC,&end)==0);
    CHECK((end.tv_sec-start.tv_sec)*1000000000LL+end.tv_nsec-start.tv_nsec>=30000000);
    int fd=open("/tmp/native-map",O_CREAT|O_TRUNC|O_RDWR,0600);CHECK(fd>=0);CHECK(write(fd,"abcdef",6)==6);CHECK(lseek(fd,2,SEEK_SET)==2);
    char *map=mmap(0,4096,PROT_READ|PROT_WRITE,MAP_PRIVATE,fd,0);CHECK(map!=MAP_FAILED);CHECK(!memcmp(map,"abcdef",6) && map[6]==0 && map[4095]==0);CHECK(lseek(fd,0,SEEK_CUR)==2);
    map[0]='X';CHECK(lseek(fd,0,SEEK_SET)==0);CHECK(read(fd,b,1)==1 && b[0]=='a');CHECK(munmap(map,4096)==0);
    errno=0;CHECK(mmap(0,4096,PROT_READ|PROT_WRITE|PROT_EXEC,MAP_PRIVATE,fd,0)==MAP_FAILED);
    CHECK(unlink("/tmp/native-map")==0);
    int replacement=open("/tmp/native-map",O_CREAT|O_EXCL|O_RDWR,0600);CHECK(replacement>=0);CHECK(write(replacement,"new",3)==3);
    CHECK(lseek(fd,0,SEEK_SET)==0);CHECK(read(fd,b,6)==6 && !memcmp(b,"abcdef",6));CHECK(close(fd)==0);
    CHECK(lseek(replacement,0,SEEK_SET)==0);CHECK(read(replacement,b,3)==3 && !memcmp(b,"new",3));CHECK(close(replacement)==0);CHECK(unlink("/tmp/native-map")==0);
    puts("[NATIVE-FD] unlinked open identity PASS");
    CHECK(pipe(p)==0 && pipe(q)==0);posix_spawn_file_actions_t fa;CHECK(posix_spawn_file_actions_init(&fa)==0);
    CHECK(posix_spawn_file_actions_adddup2(&fa,p[0],0)==0);CHECK(posix_spawn_file_actions_adddup2(&fa,q[1],1)==0);
    CHECK(posix_spawn_file_actions_addclose(&fa,p[1])==0);CHECK(posix_spawn_file_actions_addclose(&fa,q[0])==0);
    char *args[]={"musl_native_fd","child",0};pid_t child;CHECK(posix_spawn(&child,"/app/musl_native_fd",&fa,0,args,0)==0);CHECK(posix_spawn_file_actions_destroy(&fa)==0);
    CHECK(close(p[0])==0 && close(q[1])==0);CHECK(write(p[1],"ping",4)==4);CHECK(close(p[1])==0);CHECK(read(q[0],b,4)==4 && !memcmp(b,"pong",4));CHECK(close(q[0])==0);
    int status;CHECK(waitpid(child,&status,0)==child && WIFEXITED(status) && WEXITSTATUS(status)==7);
    CHECK(pipe(q)==0);CHECK(posix_spawn_file_actions_init(&fa)==0);CHECK(posix_spawn_file_actions_adddup2(&fa,q[1],1)==0);CHECK(posix_spawn_file_actions_addclose(&fa,q[0])==0);
    args[1]="delay";CHECK(posix_spawn(&child,"/app/musl_native_fd",&fa,0,args,0)==0);CHECK(posix_spawn_file_actions_destroy(&fa)==0);CHECK(close(q[1])==0);
    pollfd.fd=q[0];pollfd.events=POLLIN;CHECK(poll(&pollfd,1,1000)==1 && (pollfd.revents&POLLIN));CHECK(read(q[0],b,1)==1 && b[0]=='x');CHECK(close(q[0])==0);CHECK(waitpid(child,&status,0)==child && WEXITSTATUS(status)==0);
    puts("[NATIVE-FD] flags, shared status, poll sleep/wake, private mmap, spawn pipes PASS");return 0;
}
