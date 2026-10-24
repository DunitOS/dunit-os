#include <unistd.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#define CHECK(x) do { if(!(x)){printf("[LIBC-DURABLE] FAIL line=%d\n",__LINE__);return 1;} } while(0)
int main(int argc,char **argv) {
    CHECK(argc==2);
    int saving=!strcmp(argv[1],"write");
    CHECK(saving || !strcmp(argv[1],"verify"));
    int fd=open("/persist/libc-durable.dat",saving?(O_WRONLY|O_CREAT|O_TRUNC):O_RDONLY,0600);CHECK(fd>=0);
    unsigned char block[1024];uint64_t hash=14695981039346656037ULL;
    for(unsigned offset=0;offset<131072;offset+=sizeof block){
        if(saving)for(unsigned i=0;i<sizeof block;i++)block[i]=(unsigned char)((offset+i)*17+31);
        size_t done=0;
        while(done<sizeof block){ssize_t n=saving?write(fd,block+done,sizeof block-done):read(fd,block+done,sizeof block-done);CHECK(n>0);done+=n;}
        for(unsigned i=0;i<sizeof block;i++){
            CHECK(block[i]==(unsigned char)((offset+i)*17+31));
            hash=(hash^block[i])*1099511628211ULL;
        }
    }
    CHECK(hash==0xb4c071280fb62325ULL);
    if(saving)CHECK(fsync(fd)==0);else CHECK(read(fd,block,1)==0);
    CHECK(close(fd)==0);
    printf("[LIBC-DURABLE] %s bytes=131072 hash=%016llx\n",saving?"saved":"verified",(unsigned long long)hash);
    return 0;
}
