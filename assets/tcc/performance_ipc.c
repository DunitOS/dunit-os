/* Fixed two-process native IPC round trips. Trial 0 warmup, then five samples. */
#include <dunit/syscall.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <spawn.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
#define ROUNDS 1000
extern char **environ;
struct Packet {uint64_t sequence,trial,payload,checksum;};
static long call3(long n,long a,long b,long c) {
    long result;
    __asm__ volatile("syscall":"=a"(result):"a"(n),"D"(a),"S"(b),"d"(c):"rcx","r11","memory");
    return result;
}
static uint64_t now(void) {
    struct timespec t;if(clock_gettime(CLOCK_MONOTONIC,&t))exit(2);
    return (uint64_t)t.tv_sec*1000000000ull+t.tv_nsec;
}
static void fail(const char *stage) {printf("[PERF-IPC] FAIL %s\n",stage);exit(3);}
static uint64_t hash(struct Packet *p) {return (p->sequence*16777619ull)^(p->trial*0x9e3779b9ull)^p->payload;}
static void send_packet(unsigned peer,struct Packet *p) {
    if(call3(SYS_SEND_MESSAGE,peer,(long)p,sizeof *p)!=sizeof *p)fail("send");
}
static void receive_packet(unsigned peer,struct Packet *p) {
    uint64_t deadline=now()+10000000000ull;
    for(;;) {
        unsigned sender=0;long result=call3(SYS_RECEIVE_MESSAGE_FROM,(long)p,sizeof *p,(long)&sender);
        if(result==sizeof *p) {if(sender!=peer || p->checksum!=hash(p))fail("identity/data");return;}
        if(result!=-11 || now()>=deadline)fail("receive/timeout");
        if(call3(SYS_WAIT_EVENT,1000,0,0)<0)fail("wait");
    }
}
static int child(unsigned parent) {
    unsigned trial,i;struct Packet p;
    for(trial=0;trial<6;trial++)for(i=0;i<ROUNDS;i++) {
        receive_packet(parent,&p);
        if(p.sequence!=i || p.trial!=trial || p.payload!=0x123456789abcdef0ull+i)fail("request order");
        p.payload^=0xfedcba9876543210ull;p.checksum=hash(&p);send_packet(parent,&p);
    }
    return 0;
}
int main(int argc,char **argv) {
    setvbuf(stdout,0,_IONBF,0);
    if(argc==3 && argv[1][0]=='c')return child((unsigned)strtoul(argv[2],0,10));
    char parent[32];snprintf(parent,sizeof parent,"%u",(unsigned)getpid());
    char *args[]={"performance-ipc","child",parent,0};pid_t pid=0;
    if(posix_spawn(&pid,"/tmp/performance-ipc",0,0,args,environ))fail("spawn");
    unsigned trial,i;
    for(trial=0;trial<6;trial++) {
        uint64_t start=now(),sum=0,expected=0;
        for(i=0;i<ROUNDS;i++) {
            struct Packet p={(uint64_t)i,(uint64_t)trial,0x123456789abcdef0ull+i,0};p.checksum=hash(&p);
            send_packet((unsigned)pid,&p);receive_packet((unsigned)pid,&p);
            if(p.sequence!=i || p.trial!=trial || p.payload!=((0x123456789abcdef0ull+i)^0xfedcba9876543210ull))fail("reply order");
            sum+=p.checksum;
            expected+=((uint64_t)i*16777619ull)^((uint64_t)trial*0x9e3779b9ull)^((0x123456789abcdef0ull+i)^0xfedcba9876543210ull);
        }
        uint64_t elapsed=now()-start;if(sum!=expected)fail("checksum");
        char record[256];
        int length=snprintf(record,sizeof record,"[PERF-IPC] {\"trial\":%u,\"elapsed_ns\":%llu,\"round_trips\":%u,\"bytes\":%u,\"checksum\":%llu}\n",trial,(unsigned long long)elapsed,ROUNDS,(unsigned)(ROUNDS*sizeof(struct Packet)*2),(unsigned long long)sum);
        if(length<0 || length>=sizeof record || write(1,record,length)!=length)fail("report");
    }
    int status=0;if(waitpid(pid,&status,0)!=pid || !WIFEXITED(status) || WEXITSTATUS(status)!=0)fail("child exit");
    puts("[PERF-IPC] all checks passed");return 0;
}
