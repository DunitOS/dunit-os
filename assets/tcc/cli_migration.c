/* Capture actual stdout/stderr through native spawn actions, not serial tags. */
#include <dunit.h>
#include <sys/wait.h>

static char **environment;
static const char *out_path="/tmp/sdk-cli.stdout", *err_path="/tmp/sdk-cli.stderr";
static int capture(char *const args[], int code, char **out, char **err) {
    posix_spawn_file_actions_t actions;pid_t pid;int status,e;
    if((e=posix_spawn_file_actions_init(&actions)))return e;
    e=posix_spawn_file_actions_addopen(&actions,1,out_path,O_WRONLY|O_CREAT|O_TRUNC,0600);
    if(!e)e=posix_spawn_file_actions_addopen(&actions,2,err_path,O_WRONLY|O_CREAT|O_TRUNC,0600);
    if(!e)e=posix_spawn(&pid,args[0],&actions,NULL,args,environment);
    posix_spawn_file_actions_destroy(&actions);if(e)return e;
    if(waitpid(pid,&status,0)!=pid||!WIFEXITED(status)||WEXITSTATUS(status)!=code)return EIO;
    const char *paths[]={out_path,err_path};char **dest[]={out,err};
    for(unsigned i=0;i<2;i++){
        FILE *f=fopen(paths[i],"rb");if(!f)return errno;
        char *data=malloc(65537);if(!data){fclose(f);return ENOMEM;}
        size_t n=fread(data,1,65537,f);int failed=ferror(f);fclose(f);
        if(failed||n>65536){free(data);return EIO;}data[n]=0;*dest[i]=data;
    }
    return 0;
}
int main(int argc,char **argv,char **envp){
    (void)argc;(void)argv;environment=envp;
    setvbuf(stdout,NULL,_IONBF,0);
    char *out=NULL,*err=NULL;int e;
    char *fetch[]={"/app/dufetch",NULL};
    if((e=capture(fetch,0,&out,&err)))goto fail;
    if(!*out||*err){e=EIO;goto fail;}
    size_t n=strlen(out);char line[100];const char *hex="0123456789abcdef";
    for(size_t at=0;at<n;at+=32){
        size_t take=n-at<32?n-at:32;memcpy(line,"[CLI-REPORT] ",13);
        for(size_t j=0;j<take;j++){unsigned byte=(unsigned char)out[at+j];line[13+j*2]=hex[byte>>4];line[14+j*2]=hex[byte&15];}
        line[13+take*2]=0;puts(line);
    }
    free(out);free(err);out=err=NULL;
    char *wc[]={"/app/wc_demo",NULL};
    if((e=capture(wc,0,&out,&err)))goto fail;
    if(strcmp(out,"wc demo.txt -> 3 lines, 13 words, 61 bytes\n")||*err){e=EIO;goto fail;}
    free(out);free(err);out=err=NULL;puts("[CLI-MIGRATION] wc default exact stdout");
    char *path[]={"/app/wc_demo","/tmp/m711-wc.txt",NULL};
    if((e=capture(path,0,&out,&err)))goto fail;
    if(strcmp(out,"wc /tmp/m711-wc.txt -> 3 lines, 13 words, 61 bytes\n")||*err){e=EIO;goto fail;}
    free(out);free(err);out=err=NULL;puts("[CLI-MIGRATION] wc explicit path exact stdout");
    char *bad[]={"/app/wc_demo","/missing-parent/m711-wc.txt",NULL};
    if((e=capture(bad,1,&out,&err)))goto fail;
    if(*out||strcmp(err,"fopen(w): No such file or directory\n")){e=EIO;goto fail;}
    free(out);free(err);puts("[CLI-MIGRATION] wc exact stderr and exit 1");
    puts("[CLI-MIGRATION] ok");return 0;
fail:free(out);free(err);printf("[CLI-MIGRATION] FAIL %d\n",e);return 1;
}
