#include "capture_bridge.h"
#include <cwchar>
#include <iostream>
#include <thread>
#include <array>
#include <algorithm>
int main(int argc, char** argv) {
    if(argc!=2 || std::string(argv[1])!="--reject-unknown") return 2;
    if(TxCaptureAbi()!=1) return 3;
    void* owner=TxCaptureCreate(); if(!owner) return 4;
    const wchar_t mic[]=L"tingxian-nonexistent-input", ret[]=L"tingxian-nonexistent-return";
    TxCaptureFormat f{};TxCaptureStatus s{};
    const auto denied=TxCapturePrepare(owner,mic,UINT32(wcslen(mic)),ret,UINT32(wcslen(ret)),1,0,&f,sizeof(f));
    const auto unknown=TxCaptureInspect(owner,mic,UINT32(wcslen(mic)),ret,UINT32(wcslen(ret)));
    std::array<float,4800> pcm;pcm.fill(.5f);TxCapturePacket p{};
    const auto idle_read=TxCaptureRead(owner,1,pcm.data(),UINT32(pcm.size()),&p,sizeof(p));
    const auto idle_start=TxCaptureStart(owner,1);
    const auto status=TxCaptureStatusRead(owner,&s,sizeof(s));
    HRESULT foreign=S_OK;std::thread other([&]{foreign=TxCaptureDestroy(owner);});other.join();
    const auto destroyed=TxCaptureDestroy(owner);
    if(denied!=E_ACCESSDENIED || !FAILED(unknown) || !FAILED(idle_read) || !FAILED(idle_start) ||
        p.frames || !std::all_of(pcm.begin(),pcm.end(),[](float f){return f==0;}) || status!=S_OK || s.phase!=0 ||
        s.delivered_frames || s.packets || foreign!=RPC_E_WRONG_THREAD || destroyed!=S_OK) return 5;
    std::cout<<"PASS capture production ABI/denied/unknown selection/owner lifetime; no capture initialized or started\n";
    return 0;
}
