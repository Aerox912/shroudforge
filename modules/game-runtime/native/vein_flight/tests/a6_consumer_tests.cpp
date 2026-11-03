#include <iostream>
#if !__has_include("vein_flight/a6_consumer.hpp")
int main() { std::cerr << "FAIL: assigned A6 consumer is not implemented\n"; return 1; }
#else
#include "vein_flight/a6_consumer.hpp"
#include <windows.h>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

using VeinFlight::A6Consumer;
using namespace vein;
using namespace vein::ikea_client;
namespace {
unsigned checks = 0, failures = 0;
void check(bool ok, const char* name) { ++checks; if (!ok) { ++failures; std::cerr << "FAIL: " << name << '\n'; } }
void put(const std::filesystem::path& p, const std::string& s) { std::ofstream f(p, std::ios::binary); f << s; }
std::string get(const std::filesystem::path& p) { std::ifstream f(p, std::ios::binary); return {std::istreambuf_iterator<char>(f), {}}; }
InputObservation input(bool activation=false, bool radius=false, Focus focus=Focus::gameplay, Capture capture=Capture::clear, unsigned key=119, unsigned radius_key=123) {
    InputObservation o{}; o.focus=focus; o.capture=capture;
    if(activation) o.keys[key/64] |= std::uint64_t{1} << (key%64);
    if(radius) o.keys[radius_key/64] |= std::uint64_t{1} << (radius_key%64);
    return o;
}
struct Packet {
    unsigned calls=0; std::uint32_t magic=0, encoded=0; PacketStatus result=PacketStatus::written;
    static PacketStatus commit(void* ctx, std::uint32_t m, std::uint32_t e) noexcept {
        auto& p=*static_cast<Packet*>(ctx); ++p.calls; p.magic=m; p.encoded=e; return p.result;
    }
    PacketTarget target() { return {this,commit}; }
};
struct Files {
    unsigned calls=0; bool null_backup=true, fail=false, ambiguous=false, erase_after=false;
    static settings_file::ReplaceResult replace(void* ctx,const std::filesystem::path& target,const std::filesystem::path& stage,const std::filesystem::path* backup) noexcept {
        auto& s=*static_cast<Files*>(ctx); ++s.calls; s.null_backup = s.null_backup && backup==nullptr;
        if(s.fail) {
            if(s.ambiguous) { try { put(target,"external-ambiguous-result\n"); } catch(...) {} }
            return {false,ERROR_WRITE_FAULT};
        }
        const bool ok=ReplaceFileW(target.c_str(),stage.c_str(),nullptr,0,nullptr,nullptr)!=FALSE;
        if(ok&&s.erase_after) DeleteFileW(target.c_str());
        return {ok,ok?0u:GetLastError()};
    }
    settings_file::FileOperations ops() { return {this,replace}; }
};
std::string settings(const char* mode="toggle", unsigned radius=3, unsigned key=119, unsigned radius_key=123) {
    return std::string("version=1\nmode=")+mode+"\nkey="+std::to_string(key)+"\nradius="+std::to_string(radius)+"\ntoggle_migration_revision=9\nradius_key="+std::to_string(radius_key)+"\n";
}
void path_and_preservation(const std::filesystem::path& root) {
    A6Consumer relative; auto r=relative.initialize("kfc-runtime.ini");
    check(!r&&r.error==settings_file::Error::invalid_path,"relative provider path refused");
    check(!relative.begin_activation(1),"failed path cannot activate");
    const auto absent=root/"missing.ini"; A6Consumer missing; r=missing.initialize(absent);
    check(!r&&!std::filesystem::exists(absent),"missing admitted file refused without defaults or creation");
    const auto invalid=root/"invalid.ini"; put(invalid,"mode=toggle\nkey=0\nradius=3\n"); A6Consumer bad;
    check(!bad.initialize(invalid)&&get(invalid)=="mode=toggle\nkey=0\nradius=3\n","unsupported unbound value preserved and refused");
    const auto path=root/"preserved.ini"; const auto original=settings(); put(path,original); A6Consumer c;
    check(bool(c.initialize(path))&&get(path)==original,"valid settings load is byte preserving");
    const auto v=c.settings(); check(v&&v->mode==Mode::Toggle&&v->activation_key==119&&v->radius_key==123&&v->radius_multiplier==3&&v->toggle_migration_revision==9,"mode keys F12 radius revision preserved");
    check(!c.begin_activation(0)&&c.begin_activation(10),"nonzero activation only");
    check(!c.initialize(root/"other.ini")&&!std::filesystem::exists(root/"other.ini"),"canonical path cannot be rebound after initialization");
    c.retire_activation(9); Packet p; (void)c.observe_worker_input(10,1,input(),100); (void)c.observe_worker_input(10,1,input(true),101);
    check(c.on_sampled_packet(10,1,p.target(),input(),false,102).status==PacketStatus::written,"stale retire does not close current activation");
    c.retire_activation(10); check(c.on_sampled_packet(10,1,p.target(),input(),false,103).status!=PacketStatus::written,"deactivation disarms consumer");
}
void controls_and_packets(const std::filesystem::path& root) {
    const auto path=root/"toggle.ini"; put(path,settings()); A6Consumer c; Packet p;
    check(bool(c.initialize(path))&&c.begin_activation(1),"toggle consumer begins");
    (void)c.observe_worker_input(1,7,input(true),100);
    check(c.on_sampled_packet(1,7,p.target(),input(),false,101).status==PacketStatus::inactive,"held toggle at activation is not a new edge");
    (void)c.observe_worker_input(1,7,input(),102); (void)c.observe_worker_input(1,7,input(true),103);
    check(c.on_sampled_packet(1,7,p.target(),input(),false,104).status==PacketStatus::written&&p.magic==namespace_magic&&(p.encoded&0xff000000)==tag&&((p.encoded>>22)&3)==2,"fresh toggle writes exact A6 radius3 wire");
    const auto serial=c.last_serial(); p.result=PacketStatus::occupied;
    check(c.on_sampled_packet(1,7,p.target(),input(),false,105).status==PacketStatus::occupied&&c.last_serial()==serial,"occupied packet does not consume serial");
    p.result=PacketStatus::unavailable; check(c.on_sampled_packet(1,7,p.target(),input(),false,106).status==PacketStatus::unavailable&&c.last_serial()==serial,"failed native commit does not consume serial");
    p.result=PacketStatus::written;
    check(c.on_sampled_packet(1,7,p.target(),input(),false,135).status==PacketStatus::written,"sample age32 admitted");
    const auto after=c.last_serial(); check(c.on_sampled_packet(1,7,p.target(),input(),false,136).status==PacketStatus::inactive&&c.last_serial()==after,"sample age33 refused without serial");
    (void)c.observe_worker_input(1,7,input(),200); (void)c.observe_worker_input(1,7,input(true),201); c.reset_native_session(1);
    check(c.on_sampled_packet(1,7,p.target(),input(),false,202).status==PacketStatus::inactive,"native reset discards toggle and sample");
    (void)c.observe_worker_input(1,8,input(),203); (void)c.observe_worker_input(1,8,input(true),204);
    check(c.on_sampled_packet(1,7,p.target(),input(),false,205).status==PacketStatus::inactive,"wrong generation is refused");
    const auto hold_path=root/"hold.ini"; put(hold_path,settings("hold",2,192)); A6Consumer h; check(bool(h.initialize(hold_path))&&h.begin_activation(2),"Hold preserves grave binding");
    (void)h.observe_worker_input(2,1,input(true,false,Focus::gameplay,Capture::clear,192),300);
    check(h.on_sampled_packet(2,1,p.target(),input(),false,301).status==PacketStatus::inactive,"Hold release rechecked at packet boundary");
    check(h.on_sampled_packet(2,1,p.target(),input(),true,302).status==PacketStatus::written,"fresh Hold remains active");
    check(h.on_sampled_packet(2,1,p.target(),input(false,false,Focus::other),true,303).status==PacketStatus::inactive,"focus loss refuses Hold");
    const auto always_path=root/"always.ini"; put(always_path,settings("always",1)); A6Consumer a; check(bool(a.initialize(always_path))&&a.begin_activation(3),"Always initialized");
    (void)a.observe_worker_input(3,1,input(),400);
    check(a.on_sampled_packet(3,1,p.target(),input(),false,401).status==PacketStatus::written,"Always works without activation key");
    check(a.on_sampled_packet(3,1,p.target(),input(false,false,Focus::gameplay,Capture::captured),false,402).status==PacketStatus::inactive,"capture gates Always");
}
void radius_and_failures(const std::filesystem::path& root) {
    const auto path=root/"radius.ini"; put(path,settings("toggle",3,123,123)); Files f; A6Consumer c;
    check(bool(c.initialize(path,f.ops()))&&c.begin_activation(5),"explicit shared binding preserved");
    (void)c.observe_worker_input(5,1,input(false,false,Focus::gameplay,Capture::clear,123,123),10);
    auto changed=c.observe_worker_input(5,1,input(true,true,Focus::gameplay,Capture::clear,123,123),11);
    check(changed.active&&changed.radius_changed&&changed.radius_multiplier==1,"same fresh F12 binding can toggle and wrap radius");
    check(c.settings_pending()&&!c.take_notices(5).radius,"radius is not claimed saved before persistence");
    check(c.flush_settings()==settings_file::Error::none&&f.calls==1&&f.null_backup,"one changed radius saved without old-settings backup");
    const auto saved=parse_settings(get(path)); check(saved.valid&&saved.settings.activation_key==123&&saved.settings.radius_key==123&&saved.settings.radius_multiplier==1&&saved.settings.mode==Mode::Toggle&&saved.settings.toggle_migration_revision==9,"save preserves mode keys and revision");
    check(c.take_notices(5).radius==1u&&!c.settings_pending(),"saved radius notice follows successful persistence");
    check(c.flush_settings()==settings_file::Error::none&&f.calls==1,"idle flush performs no replacement");
    for(const bool ambiguous:{false,true}) {
        const auto failpath=root/(ambiguous?"ambiguous.ini":"failure.ini"); const auto original=settings(); put(failpath,original); Files x; x.fail=true; x.ambiguous=ambiguous; A6Consumer failed;
        check(bool(failed.initialize(failpath,x.ops()))&&failed.begin_activation(6),"failure fixture initialized");
        (void)failed.observe_worker_input(6,2,input(),20); (void)failed.observe_worker_input(6,2,input(false,true),21);
        check(failed.flush_settings()!=settings_file::Error::none&&x.calls==1&&failed.faulted(),"replacement error disarms consumer");
        check(failed.flush_settings()!=settings_file::Error::none&&x.calls==1,"failed persistence has no automatic retry");
        check(!failed.take_notices(6).radius&&!failed.begin_activation(7),"failed save cannot announce saved or silently reactivate");
        Packet p; check(failed.on_sampled_packet(6,2,p.target(),input(),false,22).status!=PacketStatus::written&&p.calls==0,"persistence fault blocks mining publication only");
        check(get(failpath)==(ambiguous?"external-ambiguous-result\n":original),"failure never performs restoration or overwrite");
    }
    const auto conflict_path=root/"conflict.ini"; put(conflict_path,settings()); Files conflict_ops; A6Consumer conflict;
    check(bool(conflict.initialize(conflict_path,conflict_ops.ops()))&&conflict.begin_activation(8),"conflict fixture initialized");
    (void)conflict.observe_worker_input(8,1,input(),30); (void)conflict.observe_worker_input(8,1,input(false,true),31); put(conflict_path,"external-change\n");
    check(conflict.flush_settings()==settings_file::Error::conflict&&conflict_ops.calls==0&&get(conflict_path)=="external-change\n"&&conflict.faulted(),"stale source refuses replacement without overwrite");
    const auto gone_path=root/"gone-after-replace.ini"; put(gone_path,settings()); Files gone_ops; gone_ops.erase_after=true; A6Consumer gone;
    check(bool(gone.initialize(gone_path,gone_ops.ops()))&&gone.begin_activation(9),"post-replacement fixture initialized");
    (void)gone.observe_worker_input(9,1,input(),40); (void)gone.observe_worker_input(9,1,input(false,true),41);
    check(gone.flush_settings()!=settings_file::Error::none&&gone.faulted()&&!gone.take_notices(9).radius,"missing post-replacement file must not be claimed saved");
    const auto default_path=root/"default-ops.ini"; put(default_path,settings()); A6Consumer default_ops;
    check(bool(default_ops.initialize(default_path))&&default_ops.begin_activation(10),"default OS replacement initialized");
    (void)default_ops.observe_worker_input(10,1,input(),50); (void)default_ops.observe_worker_input(10,1,input(false,true),51);
    check(default_ops.flush_settings()==settings_file::Error::none&&parse_settings(get(default_path)).settings.radius_multiplier==1,"default backup-free OS replacement saves current radius");
    const auto readonly_path=root/"readonly.ini"; const auto readonly_bytes=settings(); put(readonly_path,readonly_bytes); A6Consumer readonly;
    check(bool(readonly.initialize(readonly_path))&&readonly.begin_activation(11),"read-only failure fixture initialized");
    (void)readonly.observe_worker_input(11,1,input(),60); (void)readonly.observe_worker_input(11,1,input(false,true),61);
    check(SetFileAttributesW(readonly_path.c_str(),FILE_ATTRIBUTE_READONLY)!=FALSE,"fixture read-only attribute set");
    check(readonly.flush_settings()==settings_file::Error::access_denied&&readonly.faulted()&&get(readonly_path)==readonly_bytes,"read-only settings are preserved and consumer disarmed");
    check(SetFileAttributesW(readonly_path.c_str(),FILE_ATTRIBUTE_NORMAL)!=FALSE,"fixture read-only attribute restored");
    for(const auto& e:std::filesystem::directory_iterator(root)) check(e.path().extension()!=L".bak"&&e.path().extension()!=L".tmp","no backup or temporary output retained");
}

void text_preservation(const std::filesystem::path& root) {
    struct Case { std::string before, after; unsigned radius; };
    const std::vector<Case> cases{
        {"# personal comment\nmode=always\nextra=first\nradius = 3  \nextra=second\nradius_key=123\n# footer\n",
         "# personal comment\nmode=always\nextra=first\nradius = 1  \nextra=second\nradius_key=123\n# footer\n",3},
        {"\xef\xbb\xbf" "version=1\r\n# caf\xc3\xa9\r\nmode=always\r\ncustom=\xc3\xa9\r\n\t radius\t=\t003\t\r\nradius_key=123",
         "\xef\xbb\xbf" "version=1\r\n# caf\xc3\xa9\r\nmode=always\r\ncustom=\xc3\xa9\r\n\t radius\t=\t001\t\r\nradius_key=123",3},
        {"\xef\xbb\xbf" "radius=3\nmode=always\nextra=same\nextra=same\n",
         "\xef\xbb\xbf" "radius=1\nmode=always\nextra=same\nextra=same\n",3},
        {"# mixed endings\nmode=always\r\nradius=3\nextra=keep\r\n",
         "# mixed endings\nmode=always\r\nradius=1\nextra=keep\r\n",3},
        {"# missing radius\r\nmode=always\r\nextra=keep\r\n",
         "# missing radius\r\nmode=always\r\nextra=keep\r\nradius=3\r\n",2},
        {"mode=always\r\nextra=no-final-newline",
         "mode=always\r\nextra=no-final-newline\r\nradius=3",2},
    };
    unsigned index=0;
    for(const auto& item:cases) {
        const auto path=root/("roundtrip-"+std::to_string(index++)+".ini"); put(path,item.before); Files f; A6Consumer c;
        const bool loaded=bool(c.initialize(path,f.ops())); check(loaded,"round-trip input loads without rewriting");
        check(get(path)==item.before,"load preserves raw unknown entries comments and encoding");
        if(!loaded) continue;
        const auto current=c.settings(); check(current&&current->radius_multiplier==item.radius,"BOM and omitted-radius semantics are explicit");
        check(c.begin_activation(30),"round-trip activation begins");
        (void)c.observe_worker_input(30,1,input(),100); const auto edge=c.observe_worker_input(30,1,input(false,true),101);
        check(edge.radius_changed,"round-trip radius edge observed");
        const auto error=c.flush_settings();
        check(error==settings_file::Error::none&&get(path)==item.after,"only radius text changes; original bytes outside value stay intact");
        check(f.calls==1&&f.null_backup,"round-trip uses one backup-free replacement");
        check(c.take_notices(30).radius==edge.radius_multiplier,"only persisted round-trip radius is announced");
    }
    const std::vector<std::string> invalids{
        "mode=always\nradius=3\nradius=3\n# duplicate known radius\n",
        "mode=always\nmode=always\nradius=3\n",
        std::string("\xff\xfev\0e\0r\0",8),
        "\xef\xbb\xbf" "version=2\nmode=always\nradius=3\n"
    };
    for(const auto& bytes:invalids) {
        const auto path=root/("unsupported-"+std::to_string(index++)+".ini"); put(path,bytes); A6Consumer c;
        check(!c.initialize(path)&&get(path)==bytes,"known duplicates unsupported encoding/version refuse without rewriting");
    }
    for(const auto& bytes:std::vector<std::string>{"mode=always", "mode=always\nextra=keep\r\n"}) {
        const auto path=root/("ambiguous-eol-"+std::to_string(index++)+".ini"); put(path,bytes); Files f; A6Consumer c;
        check(bool(c.initialize(path,f.ops()))&&c.begin_activation(31),"ambiguous append input parses but is not rewritten");
        (void)c.observe_worker_input(31,1,input(),110); (void)c.observe_worker_input(31,1,input(false,true),111);
        check(c.flush_settings()==settings_file::Error::invalid_settings&&f.calls==0&&get(path)==bytes&&c.faulted(),"missing radius with no unique newline convention refuses and preserves raw bytes");
    }
    const std::string original="# retained\nmode=always\nradius=3\nunknown=one\nunknown=two\n";
    const auto conflict_path=root/"text-conflict.ini"; put(conflict_path,original); Files conflict_ops; A6Consumer conflict;
    check(bool(conflict.initialize(conflict_path,conflict_ops.ops()))&&conflict.begin_activation(32),"text conflict initialized");
    (void)conflict.observe_worker_input(32,1,input(),120); (void)conflict.observe_worker_input(32,1,input(false,true),121);
    const auto external=original+"# external change\n"; put(conflict_path,external);
    check(conflict.flush_settings()==settings_file::Error::conflict&&conflict_ops.calls==0&&get(conflict_path)==external,"text conflict preserves unknown entries and concurrent editor bytes");
    const auto failure_path=root/"text-failure.ini"; put(failure_path,original); Files failed_ops; failed_ops.fail=true; A6Consumer failed;
    check(bool(failed.initialize(failure_path,failed_ops.ops()))&&failed.begin_activation(33),"text replacement failure initialized");
    (void)failed.observe_worker_input(33,1,input(),130); (void)failed.observe_worker_input(33,1,input(false,true),131);
    check(failed.flush_settings()!=settings_file::Error::none&&failed.faulted()&&get(failure_path)==original,"failed text replacement preserves unknown entries and disarms");
    check(failed.flush_settings()!=settings_file::Error::none&&failed_ops.calls==1&&!failed.take_notices(33).radius,"failed text replacement has no retry or saved notice");
    const auto wrong_change=root/"non-radius-change.ini"; put(wrong_change,original); Files wrong_ops; settings_file::SettingsFile file(wrong_change,wrong_ops.ops());
    const auto loaded=file.load(); check(bool(loaded),"non-radius guard fixture loaded");
    if(loaded) { auto settings=*loaded.settings; settings.activation_key=120;
        check(file.save(settings)==settings_file::Error::invalid_settings&&wrong_ops.calls==0&&get(wrong_change)==original,"writer refuses unrelated controlled-field changes"); }
}
}
int main(int argc,char** argv) {
    if(argc!=2) { std::cerr<<"explicit disposable E: fixture root required\n"; return 2; }
    const std::filesystem::path root=std::filesystem::absolute(argv[1]); std::filesystem::create_directories(root);
    path_and_preservation(root); controls_and_packets(root); radius_and_failures(root); text_preservation(root);
    std::cout<<checks<<" consumer checks, "<<failures<<" failures\n"; return failures?1:0;
}
#endif
