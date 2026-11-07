#include "timed_bridge.h"
#include <iostream>
#include <stdexcept>
#include <thread>
using namespace tingxian::video;
namespace {
int groups=0;
void expect(bool ok,const char* reason){if(!ok)throw std::runtime_error(reason);}
void check(HRESULT hr,const char* reason){if(FAILED(hr)){std::cerr<<reason<<" hr="<<std::hex<<unsigned(hr)<<std::dec<<'\n';throw std::runtime_error(reason);}}
void pass(const char* name){++groups;std::cout<<"PASS "<<name<<'\n';}
struct Owner {
    void* raw=TxTimedCreate(); VideoPipeEndpoint endpoint{};
    Owner(){expect(raw,"create");}
    ~Owner(){TxTimedStop(raw);TxTimedDestroy(raw);}
    void open(UINT64 generation=1){check(TxTimedOpen(raw,generation,reinterpret_cast<BYTE*>(&endpoint),sizeof(endpoint)),"open");}
    TimedOutputStatus status(){TimedOutputStatus value;check(TxTimedStatus(raw,&value,sizeof(value)),"status");return value;}
    template<class F>void wait(F predicate){
        HANDLE event=nullptr;check(TxTimedEvent(raw,&event),"event");const auto end=GetTickCount64()+2000;
        while(!predicate()){expect(GetTickCount64()<end,"notification deadline");WaitForSingleObject(event,50);}
    }
    TimedTicket reserve(){TimedTicket t;check(TxTimedReserve(raw,1,&t,sizeof(t)),"reserve");expect(t.owner&&t.serial&&t.generation==1,"ticket");return t;}
    TimedReceipt receipt(TimedTicket t){TimedReceipt r;HRESULT hr=S_FALSE;wait([&]{hr=TxTimedTake(raw,&t,sizeof(t),&r,sizeof(r));return hr!=S_FALSE;});check(hr,"receipt");return r;}
};
struct Peer {std::shared_ptr<FrameMailbox> box=std::make_shared<FrameMailbox>();PipeReceiver pipe{box};
    explicit Peer(Owner& o){check(pipe.connect(o.endpoint),"peer connect");o.wait([&]{return o.status().phase==TimedOutputPhase::ready;});}};
TimedFrame plan(UINT64 id=1){UINT64 now=0;expect(qpc_now(now),"QPC");return {id,now,now+800000};}
HRESULT stage(Owner& o,TimedTicket t,const TimedFrame& f,const std::vector<BYTE>& pixels){
    return TxTimedStage(o.raw,&t,sizeof(t),&f,sizeof(f),pixels.data(),UINT32(pixels.size()));
}
struct SlowPeer {
    HANDLE raw=INVALID_HANDLE_VALUE;
    struct Header{UINT32 magic=0x31565854;UINT16 version=2,kind=0;UINT32 bytes=0,reserved=0;UINT64 epoch=0,id=0,captured=0;};
    void write(const void* data,DWORD size){DWORD n=0;expect(WriteFile(raw,data,size,&n,nullptr)&&n==size,"peer write");}
    void read(void* data,DWORD size){DWORD available=0;const auto end=GetTickCount64()+2000;
        for(;;){expect(PeekNamedPipe(raw,nullptr,0,nullptr,&available,nullptr),"peer closed");if(available>=size)break;
            expect(GetTickCount64()<end,"peer read deadline");Sleep(1);}
        DWORD n=0;expect(ReadFile(raw,data,size,&n,nullptr)&&n==size,"peer read");}
    explicit SlowPeer(Owner& o){
        raw=CreateFileW(o.endpoint.name,GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_EXISTING,SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION,nullptr);
        expect(raw!=INVALID_HANDLE_VALUE,"slow peer connect");
        Header h;h.kind=1;h.bytes=32;write(&h,sizeof(h));write(o.endpoint.capability,32);read(&h,sizeof(h));expect(h.kind==2,"hello ack");
        read(&h,sizeof(h));expect(h.kind==3&&h.epoch==1,"begin");
        h.kind=7;h.bytes=4;write(&h,sizeof(h));HRESULT result=S_OK;write(&result,sizeof(result));
        o.wait([&]{return o.status().phase==TimedOutputPhase::ready;});
    }
    ~SlowPeer(){if(raw!=INVALID_HANDLE_VALUE)CloseHandle(raw);}
};
void run(){
    const std::vector<BYTE> pixels(FrameBytes,160);
    {
        Owner o;expect(TxTimedAbi()==2&&o.status().phase==TimedOutputPhase::idle,"idle ABI");o.open();
        TimedTicket waiting;expect(TxTimedReserve(o.raw,1,&waiting,sizeof(waiting))==S_FALSE&&!waiting.owner,"unconnected reserved");
        auto start=GetTickCount64();check(TxTimedStop(o.raw),"waiting stop");
        expect(GetTickCount64()-start<1000,"accept wait not cancelled");
        expect(o.status().local_clear&&!o.status().external_drain,"stop fabricated external drain");
        expect(FAILED(TxTimedOpen(o.raw,2,reinterpret_cast<BYTE*>(&o.endpoint),sizeof(o.endpoint))),"stopped object reopened");
        pass("idle/explicit open and accept cancellation without a device");
    }
    {
        Owner o;o.open();Peer peer(o);auto t=o.reserve();TimedTicket second;
        expect(TxTimedReserve(o.raw,1,&second,sizeof(second))==S_FALSE,"second reservation accepted");
        HRESULT wrong=S_OK;std::thread thread([&]{wrong=TxTimedReserve(o.raw,1,&second,sizeof(second));});thread.join();
        expect(wrong==RPC_E_WRONG_THREAD,"foreign owner thread accepted");
        auto f=plan();check(stage(o,t,f,pixels),"stage");
        expect(FAILED(stage(o,t,f,pixels)),"ticket staged twice");
        auto r=o.receipt(t);expect(r.outcome==Admission::admitted&&r.error==S_OK,"admission receipt");
        expect(TxTimedTake(o.raw,&t,sizeof(t),&r,sizeof(r))==E_INVALIDARG,"receipt replay");
        expect(peer.box->timed_metrics().accepted==1,"actual receiver did not admit");
        std::vector<BYTE> out(FrameBytes);TimedSample stamp;LONGLONG pts=0;
        check(peer.box->write_sample(out.data(),Width,out.data(),FrameBytes,f.presentation_qpc,0,
            [&](const TimedSample* s,LONGLONG p){stamp=*s;pts=p;return S_OK;}),"sample bytes");
        expect(out==pixels&&UINT64(pts)==f.presentation_qpc&&stamp.capture_qpc==f.capture_qpc,"wrong bridge pixels/time");
        pass("single reservation, owner thread, replay rejection and actual receiver pixels/time");
    }
    {
        Owner first,second;first.open();second.open();Peer p1(first),p2(second);
        auto one=first.reserve(),two=second.reserve();auto f=plan();
        expect(one.owner!=two.owner&&FAILED(stage(second,one,f,pixels)),"foreign object ticket accepted");
        check(stage(second,two,f,pixels),"own ticket remains valid");expect(second.receipt(two).outcome==Admission::admitted,"own receipt");
        pass("tickets remain bound across concurrent instances with identical epochs/serials");
    }
    {
        Owner o;o.open();Peer peer(o);auto t=o.reserve();peer.box->stop();
        check(stage(o,t,plan(),pixels),"stage before remote rejection");auto r=o.receipt(t);
        expect(r.outcome==Admission::rejected&&FAILED(r.error),"known remote rejection marked unknown/success");
        check(TxTimedStop(o.raw),"failed stop");expect(o.status().phase==TimedOutputPhase::fault&&o.status().local_clear,"fault lost on stop");
        pass("receiver rejection remains distinct and stops sender");
    }
    {
        Owner o;o.open();Peer peer(o);auto t=o.reserve();peer.pipe.disconnect();
        check(stage(o,t,plan(),pixels),"stage on disconnected peer");auto r=o.receipt(t);
        expect(r.outcome==Admission::unknown&&FAILED(r.error),"I/O failure marked definitely rejected/admitted");
        pass("missing receipt retains unknown admission instead of retrying");
    }
    {
        Owner o;o.open();SlowPeer peer(o);auto t=o.reserve();auto f=plan();f.frame_id=0;f.capture_qpc=0;f.flags=TimedNeutral;
        check(TxTimedStage(o.raw,&t,sizeof(t),&f,sizeof(f),nullptr,0),"stage protocol probe");
        SlowPeer::Header h;peer.read(&h,sizeof(h));expect(h.kind==6&&h.bytes==sizeof(TimedFrame),"neutral wire");
        TimedFrame received;peer.read(&received,sizeof(received));h.kind=7;h.bytes=4;peer.write(&h,sizeof(h));
        HRESULT invalid=S_FALSE;peer.write(&invalid,sizeof(invalid));
        const auto r=o.receipt(t);expect(r.outcome==Admission::unknown&&FAILED(r.error),"nonzero success receipt accepted as admission");
        pass("unknown positive receipt is a protocol fault, never successful admission");
    }
    {
        Owner o;o.open();SlowPeer peer(o);auto t=o.reserve();auto f=plan();
        const auto start=GetTickCount64();check(stage(o,t,f,pixels),"bounded stage");
        expect(GetTickCount64()-start<250,"stage waited for receiver");
        SlowPeer::Header h;peer.read(&h,sizeof(h));expect(h.kind==6&&h.bytes==sizeof(TimedFrame)+FrameBytes,"scheduled wire");
        TimedTicket another;expect(TxTimedReserve(o.raw,1,&another,sizeof(another))==S_FALSE,"in-flight overwrite accepted");
        TxTimedCancel(o.raw);check(TxTimedStop(o.raw),"cancel blocked write");auto r=o.receipt(t);
        expect(r.outcome==Admission::cancelled&&o.status().local_clear&&!o.status().external_drain,"cancel falsely completed/drained");
        expect(GetTickCount64()-start<1000,"blocked write did not retire");
        pass("slow peer cannot block stage or grow queue; cancellation retires pending I/O");
    }
}
}
int main(){try{run();std::cout<<"{\"checks\":"<<groups<<",\"result\":\"pass\",\"synthetic_only\":true,\"device_registered\":false}\n";return 0;}
catch(const std::exception& e){std::cerr<<"FAIL "<<e.what()<<'\n';return 1;}}
