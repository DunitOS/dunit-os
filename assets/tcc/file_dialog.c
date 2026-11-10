#include <dunit.h>
#include <stdio.h>
#include <string.h>
#define SPAN(s) (const uint8_t *)(s), sizeof(s)-1
int main(int argc,char **argv,char **envp){
    setvbuf(stdout,NULL,_IONBF,0);
 DunitUiFileLabelsV1 labels={SPAN("Location"),SPAN("Name"),SPAN("Files"),SPAN("Back"),SPAN("Forward"),SPAN("Up"),SPAN("Refresh"),SPAN("Enter"),SPAN("Open"),SPAN("Cancel"),SPAN("Hidden"),SPAN("Filter"),SPAN("Name"),SPAN("Kind"),SPAN("Size"),SPAN("File"),SPAN("Directory"),SPAN("Device"),SPAN("Replace file?"),SPAN("This file exists. Replace?"),SPAN("Replace"),SPAN("Invalid path"),SPAN("Invalid name"),SPAN("Missing file"),SPAN("Not a directory"),SPAN("Invalid entries")};
 DunitUiStringV1 ext={SPAN("c")};
 DunitUiFileFilterV1 filter={SPAN("c"),SPAN("C sources"),&ext,1};
 DunitUiFileDialogV1 options={DUNIT_UI_ABI_VERSION,sizeof options,DUNIT_UI_FILE_OPEN,0,SPAN("picker"),SPAN("C SDK open file"),SPAN("/assets/tcc"),SPAN("workspace.c"),&filter,1,&labels};
 uint8_t path[256];size_t written=0;
 int result=dunit_ui_choose_file_v1(argc,(const char *const *)argv,(const char *const *)envp,&options,path,sizeof path,&written);
 if(result==0&&written==23&&!memcmp(path,"/assets/tcc/workspace.c",23)){puts("[C-PICKER] selected real existing filtered file");return 0;}
 printf("[C-PICKER] failure %d bytes %zu\n",result,written);return 1;
}
