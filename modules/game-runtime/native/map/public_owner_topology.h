// Included inside PublicCreativeCompat after its exact descriptors/identity API.
// One shared finite state per whole observation; no foreign graph is cached.
namespace FiniteGraph
{
    struct Limits { unsigned reads=64; std::size_t bytes=4096; };
    struct Stats { unsigned reads=0,nearCount=0,greenCount=0,creativeCount=0;std::size_t bytes=0; };
    struct Module
    {
        HMODULE reference=nullptr;std::uintptr_t base=0,callback=0;
        const Profile* profile=nullptr;FileCache* cache=nullptr;unsigned family=0;
        bool deferred=false;Digest identity{};TrustedImageBytes expected{},mapped{};
    };
    struct Observation
    {
        std::array<unsigned char,4096> bytes{};std::size_t used=0;
        Limits limits{};Stats stats{};IdentityStamp stamp{};
        std::uintptr_t renderer=0;std::array<unsigned char,18> live{};
        unsigned rootMode=0,rootKind=3,families=0,moduleCount=0;
        bool allGreenNew=true,requireNewGreen=false,complete=false;
        std::array<Module,3> modules{};
        std::array<std::uintptr_t,8> visited{};unsigned visits=0;
        explicit Observation(Limits value):limits(value),stamp(CurrentIdentityStamp()){}
        ~Observation(){for(auto& m:modules)if(m.reference)FreeLibrary(m.reference);}
        bool Read(std::uintptr_t at,void* to,std::size_t n,bool code=true)
        {
            if(stats.reads>=64 || stats.reads>=limits.reads)return false;
            ++stats.reads;
#ifdef MM_PUBLIC_OWNER_TOPOLOGY_READ
            MM_PUBLIC_OWNER_TOPOLOGY_READ(at,n);
#endif
            return Copy(at,to,n,code);
        }
        bool Add(const void* p,std::size_t n)
        {
            if(!p || used>bytes.size() || n>bytes.size()-used || used>limits.bytes || n>limits.bytes-used)return false;
            std::memcpy(bytes.data()+used,p,n);used+=n;stats.bytes=used;return true;
        }
        template<class T> bool Add(const T& x){return Add(&x,sizeof(x));}
        bool Visit(std::uintptr_t at)
        {
            if(!Range(at,1) || visits==visited.size())return false;
            for(unsigned i=0;i<visits;++i)if(visited[i]==at)return false;
            visited[visits++]=at;return true;
        }
        static bool New(const Profile* p){return p==&Find125 || p==&Teleport117 || p==&Find128 || p==&Teleport1110;}
        static bool Mov(const unsigned char* p)
        {
            if(p[0]!=0x48 || p[1]!=0xb8 || p[10]!=0xff || p[11]!=0xe0)return false;
            for(unsigned i=12;i<18;++i)if(p[i]!=0x90)return false;
            return true;
        }
        Module* Acquire(std::uintptr_t callback,bool creative)
        {
            if(moduleCount==modules.size())return nullptr;
            auto& m=modules[moduleCount++];m.callback=callback;
            if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,reinterpret_cast<LPCWSTR>(callback),&m.reference))
            {ForgetIdentity();return nullptr;}
            m.base=reinterpret_cast<std::uintptr_t>(m.reference);
            for(unsigned i=0;i+1<moduleCount;++i)if(modules[i].base==m.base)return nullptr;
            auto& dos=m.mapped.dos;auto& nt=m.mapped.nt;
            if(!Read(m.base,&dos,sizeof(dos),false) || dos.e_magic!=IMAGE_DOS_SIGNATURE || dos.e_lfanew<=0 ||
                static_cast<std::size_t>(dos.e_lfanew)>4096-sizeof(nt) ||
                !Read(m.base+dos.e_lfanew,&nt,sizeof(nt),false) || nt.Signature!=IMAGE_NT_SIGNATURE ||
                nt.FileHeader.Machine!=IMAGE_FILE_MACHINE_AMD64 || nt.OptionalHeader.Magic!=IMAGE_NT_OPTIONAL_HDR64_MAGIC)return nullptr;
            if(creative)m.profile=SelectCreativeProfile(m.base,callback,nt.FileHeader.TimeDateStamp);
            else for(const auto* p:{&Find112,&Find125,&Find128,&Teleport116,&Teleport117,&Teleport1110})
                if(callback>=m.base && callback-m.base==p->callback && nt.OptionalHeader.SizeOfImage==p->imageSize){m.profile=p;break;}
            if(!m.profile || !Range(m.base,m.profile->imageSize) || nt.OptionalHeader.SizeOfImage!=m.profile->imageSize)return nullptr;
            if(creative){if(stats.greenCount && !SupportsFiniteCreativeProfile(m.profile))return nullptr;m.cache=&fileCache;}
            else
            {
                m.family=(m.profile==&Find112 || m.profile==&Find125 || m.profile==&Find128)?1:2;
                if(families&m.family || (requireNewGreen && !New(m.profile)))return nullptr;
                families|=m.family;allGreenNew=allGreenNew&&New(m.profile);
                m.cache=m.family==1?&findFileCache:&teleportFileCache;
            }
            const Profile* actual=nullptr;
            if(!FileIdentity(m.reference,m.identity,actual,*m.cache,m.profile,&m.expected,true,&m.deferred) && !m.deferred)return nullptr;
            if(!m.deferred && actual!=m.profile)return nullptr;
            if(!Read(m.base+m.profile->body,m.mapped.body.data(),m.profile->bodySize))return nullptr;
            if(creative && (!Read(m.base+m.profile->pattern,m.mapped.pattern.data(),m.mapped.pattern.size(),false) ||
                !Read(callback,m.mapped.thunk.data(),m.mapped.thunk.size()) || m.mapped.thunk!=m.profile->thunk))return nullptr;
            return &m;
        }
        bool Approve(bool allowCold)
        {
            // ColdFileIdentity is unchanged. Reserve its fixed mapped-image
            // Copy calls (DOS/NT/body, plus Creative pattern/thunk) before ANY
            // file attempt, so they share this observation's read ceiling.
            unsigned reserved=0;
            for(unsigned i=0;i<moduleCount;++i)if(modules[i].deferred)
            {
                if(!allowCold)return false;
                reserved+=modules[i].profile->pattern?5:3;
            }
            if(stats.reads>limits.reads || reserved>64-stats.reads || reserved>limits.reads-stats.reads)return false;
            stats.reads+=reserved;
            for(unsigned i=0;i<moduleCount;++i)
            {
                auto& m=modules[i];const Profile* actual=nullptr;
                if(m.deferred && (!allowCold || !FileIdentity(m.reference,m.identity,actual,*m.cache,m.profile,&m.expected)))return false;
                if(m.deferred && actual!=m.profile)return false;
                if(std::memcmp(&m.mapped.dos,&m.expected.dos,sizeof(m.mapped.dos)) ||
                    std::memcmp(&m.mapped.nt,&m.expected.nt,sizeof(m.mapped.nt)) ||
                    std::memcmp(m.mapped.body.data(),m.expected.body.data(),m.profile->bodySize) ||
                    (m.profile->pattern && (m.mapped.pattern!=m.expected.pattern || m.mapped.thunk!=m.expected.thunk)))
                {ForgetIdentity();return false;}
                if(!Add(m.base) || !Add(m.callback) || !Add(m.identity) || !Add(m.mapped.dos) || !Add(m.mapped.nt) ||
                    !Add(m.mapped.body.data(),m.profile->bodySize) ||
                    (m.profile->pattern && (!Add(m.mapped.pattern) || !Add(m.mapped.thunk))))return false;
            }
            return IdentityStampValid(stamp);
        }
        bool Native(std::uintptr_t at,const Profile* source)
        {
            std::array<unsigned char,32> data{};constexpr unsigned char jump[6]={0xff,0x25,0,0,0,0};
            if(!Visit(at) || !Read(at,data.data(),data.size()) || std::memcmp(data.data(),Original,18))return false;
            std::uintptr_t resume=0;
            if(!std::memcmp(data.data()+18,jump,6))resume=Word<std::uintptr_t>(data.data(),24);
            else if(source==&Find112 && data[18]==0xe9)
            {if(!Relative(at+18,Word<std::int32_t>(data.data(),19),resume))return false;}
            else return false;
            const unsigned tag=4;complete=resume==renderer+18;
            return complete && Add(tag) && Add(at) && Add(data);
        }
        bool Green(std::uintptr_t at,unsigned entryMode=0,bool incomingNear=false)
        {
            if(stats.greenCount==2 || !Visit(at))return false;
            std::array<unsigned char,32> relay{};constexpr unsigned char jump[6]={0xff,0x25,10,0,0,0};
            if(!Read(at,relay.data(),relay.size()) || std::memcmp(relay.data(),jump,6))return false;
            auto* m=Acquire(Word<std::uintptr_t>(relay.data(),16),false);if(!m)return false;
            if((entryMode || incomingNear) && !New(m->profile))return false;
            if(!New(m->profile))for(unsigned i=6;i<16;++i)if(relay[i])return false;
            ++stats.greenCount;std::uintptr_t next=0;
            if(!Read(m->base+m->profile->slot,&next,sizeof(next),false))return false;
            const unsigned tag=1;
            if(!Add(tag) || !Add(m->family) || !Add(at) || !Add(relay) || !Add(next))return false;
            return Edge(next,m->profile);
        }
        bool Creative(std::uintptr_t block,unsigned kind,std::uintptr_t relayAddress=0)
        {
            if(stats.creativeCount || (stats.greenCount && !allGreenNew) || !Visit(block))return false;
            ++stats.creativeCount;std::array<unsigned char,BlockSize> code{};
            if(!Read(block,code.data(),code.size()))return false;
            auto* m=Acquire(Word<std::uintptr_t>(code.data(),0x177),true);if(!m)return false;
            std::uintptr_t object=0;std::array<unsigned char,0xd8> model{};
            if(!Read(m->base+m->profile->slot,&object,sizeof(object),false) || (object&7) || !Range(object,m->profile->objectSize) ||
                !Read(object+0x80,model.data(),model.size(),false))return false;
            const auto get=[&](std::size_t n){return Word<std::uintptr_t>(model.data(),n);};
            const auto t=get(0x20),length=get(0x28),backup=get(0x30);
            const std::size_t n=kind==2?18:length==29?10:length==30?11:0;
            if(!n || (kind!=1 && kind!=2) || length!=(kind==2?32:n+19) || get(0)!=renderer || get(8)!=block ||
                !Range(get(0x10),1) || !Range(get(0x18),1) || !Range(backup,n) || get(0x38)!=backup+n ||
                get(0x40)<get(0x38) || get(0x40)-backup>64 || get(0x48)!=length || model[0xa0]!=1 ||
                Word<unsigned>(model.data(),0xa4)!=kind || get(0xa8)!=renderer || !Range(get(0xb0),1) || !Range(get(0xb8),1) ||
                get(0xc0)!=block || get(0xc8)!=BlockSize || get(0xd0)!=m->callback ||
                Word<std::uintptr_t>(code.data(),0x17f)!=t || !Visit(t))return false;
            std::array<unsigned char,32> trampoline{};std::array<unsigned char,18> saved{};
            if(!Read(t,trampoline.data(),length) || !Read(backup,saved.data(),n,false) || std::memcmp(saved.data(),trampoline.data(),n))return false;
            constexpr unsigned char indirect[6]={0xff,0x25,0,0,0,0};
            if(kind==1)
            {
                std::uintptr_t resume=0;
                if(!Range(t,length) || relayAddress!=t+n+5 || !Visit(relayAddress) || trampoline[n]!=0xe9 ||
                    !Relative(t+n,Word<std::int32_t>(trampoline.data(),n+1),resume) || resume!=renderer+n ||
                    std::memcmp(trampoline.data()+n+5,indirect,6) || Word<std::uintptr_t>(trampoline.data(),n+11)!=block)return false;
                for(unsigned i=5;i<n;++i)if(live[i]!=0x90)return false;
                std::memcpy(saved.data()+n,live.data()+n,18-n);
            }
            else if(std::memcmp(trampoline.data()+18,indirect,6) || Word<std::uintptr_t>(trampoline.data(),24)!=renderer+18)return false;
            auto normalized=code;std::memset(normalized.data()+0x177,0,16);
            if(normalized!=m->mapped.pattern)return false;
            const bool native=!std::memcmp(saved.data(),Original,18),mov=Mov(saved.data());
            if((n==11 && !native) || (n==10 && !mov) || (!native && !mov) ||
                (kind==2 && mov && !SupportsFiniteCreativeProfile(m->profile)))return false;
            if(!stats.greenCount)rootKind=kind;
            const unsigned tag=2;
            if(!Add(tag) || !Add(kind) || !Add(n) || !Add(block) || !Add(t) || !Add(object) || !Add(relayAddress) ||
                !Add(code) || !Add(model) || !Add(saved) || !Add(trampoline) || !Add(backup))return false;
            if(native){complete=true;return true;}
            if(stats.greenCount || kind==2)requireNewGreen=true;
            return Green(Word<std::uintptr_t>(saved.data(),2));
        }
        bool Indirect(std::uintptr_t at,const Profile* source,unsigned rootEntryMode=0)
        {
            constexpr unsigned char indirect[6]={0xff,0x25,0,0,0,0},generic[6]={0xff,0x25,10,0,0,0};
            std::array<unsigned char,14> relay{};std::array<unsigned char,6> prefix{};
            if(!Read(at,relay.data(),relay.size()) || std::memcmp(relay.data(),indirect,6))return false;
            const auto destination=Word<std::uintptr_t>(relay.data(),6);
            if(!Read(destination,prefix.data(),prefix.size()))return false;
            if(!std::memcmp(prefix.data(),generic,6))
            {
                if((source && !New(source)) || stats.nearCount==2 || !Visit(at))return false;
                ++stats.nearCount;const unsigned tag=3;
                if(!Add(tag) || !Add(at) || !Add(relay))return false;
                return Green(destination,rootEntryMode,true);
            }
            // A second known near hop is not a Creative block. Do not read
            // beyond its defined14B while refusing the unsupported shape.
            if(!std::memcmp(prefix.data(),indirect,6))return false;
            if(source && (!New(source) || rootMode!=1))return false;
            return Creative(destination,1,at);
        }
        bool Edge(std::uintptr_t at,const Profile* source)
        {
            std::array<unsigned char,6> prefix{};
            if(!Read(at,prefix.data(),prefix.size()))return false;
            constexpr unsigned char generic[6]={0xff,0x25,10,0,0,0},indirect[6]={0xff,0x25,0,0,0,0};
            if(!std::memcmp(prefix.data(),Original,6))return Native(at,source);
            if(!std::memcmp(prefix.data(),generic,6))return Green(at);
            if(!std::memcmp(prefix.data(),indirect,6))return Indirect(at,source);
            if(New(source) && rootMode==2)return Creative(at,2);
            return false;
        }
        bool Capture(std::uintptr_t entry,bool allowCold)
        {
            renderer=entry;if(!Range(entry,18) || !Read(entry,live.data(),live.size()))return false;
            const unsigned tag=0;if(!Add(tag) || !Add(entry) || !Add(live))return false;
            bool ok=false;constexpr unsigned char indirect[6]={0xff,0x25,0,0,0,0},generic[6]={0xff,0x25,10,0,0,0};
            if(Mov(live.data()))ok=Green(Word<std::uintptr_t>(live.data(),2));
            else if(live[0]==0xe9)
            {
                rootMode=1;std::uintptr_t at=0;
                if(!Relative(entry,Word<std::int32_t>(live.data(),1),at))return false;
                ok=Indirect(at,nullptr,1);
            }
            else if(!std::memcmp(live.data(),indirect,6))
            {
                rootMode=2;for(unsigned i=14;i<18;++i)if(live[i]!=0x90)return false;
                const auto at=Word<std::uintptr_t>(live.data(),6);std::array<unsigned char,6> prefix{};
                if(!Read(at,prefix.data(),prefix.size()))return false;
                if(!std::memcmp(prefix.data(),indirect,6))return false;
                ok=!std::memcmp(prefix.data(),generic,6)?Green(at,2):Creative(at,2);
            }
            return ok && complete && Approve(allowCold) && IdentityStampValid(stamp);
        }
    };
    // Private SF lifetime handoff only. Stores approved module identity/lifetime,
    // never a foreign graph or an opaque cache-key-derived pointer. References
    // intentionally survive logical Stop and are retained until process exit.
    class ApprovedProviderLeases final
    {
        struct Lease { HMODULE reference{};std::uintptr_t base{},callback{};const Profile* profile{};Digest identity{}; };
        std::array<Lease,3> leases_{};unsigned count_{};bool attempted_{};IdentityStamp stamp_{};
        bool Matches(const Observation& observed) const noexcept {
            if(!count_ || count_!=observed.moduleCount || stamp_.epoch!=observed.stamp.epoch ||
               stamp_.invalidation!=observed.stamp.invalidation || !IdentityStampValid(stamp_))return false;
            for(unsigned i=0;i<count_;++i){const auto& l=leases_[i];const auto& m=observed.modules[i];
                if(l.reference!=m.reference || l.base!=m.base || l.callback!=m.callback ||
                   l.profile!=m.profile || l.identity!=m.identity)return false;
            }return true;
        }
    public:
        ApprovedProviderLeases()=default;
        ApprovedProviderLeases(const ApprovedProviderLeases&)=delete;
        ApprovedProviderLeases& operator=(const ApprovedProviderLeases&)=delete;
        bool Retained() const noexcept{return count_!=0;}
        bool Accept(const Observation& before,const Observation& after,bool allowAcquire) noexcept {
            if(count_)return Matches(before)&&Matches(after); // no extra persistent reference
            if(!allowAcquire || attempted_ || !before.moduleCount || before.moduleCount>leases_.size() ||
               before.moduleCount!=after.moduleCount || coldAdmissionEpoch!=before.stamp.epoch ||
               identityOrigin!=ProofOrigin::AdmissionUpdate || !IdentityStampValid(before.stamp))return false;
            attempted_=true;std::array<Lease,3> pending{};bool valid=true;
            for(unsigned i=0;i<before.moduleCount;++i){const auto& b=before.modules[i];const auto& a=after.modules[i];
                if(b.reference!=a.reference || b.base!=a.base || b.callback!=a.callback || b.profile!=a.profile || b.identity!=a.identity ||
                   before.stamp.epoch!=after.stamp.epoch || before.stamp.invalidation!=after.stamp.invalidation){valid=false;break;}
                auto& l=pending[i];
                if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,reinterpret_cast<LPCWSTR>(b.callback),&l.reference) ||
                   l.reference!=b.reference){valid=false;break;}
                l.base=b.base;l.callback=b.callback;l.profile=b.profile;l.identity=b.identity;
            }
            valid=valid && IdentityStampValid(before.stamp) && IdentityStampValid(after.stamp);
            if(!valid){for(auto& l:pending)if(l.reference)FreeLibrary(l.reference);return false;}
            // Both complete Observation reference sets still exist at this point.
            leases_=pending;count_=before.moduleCount;stamp_=before.stamp;return true;
        }
    };
    inline bool Inspect(std::uintptr_t entry,Proof& proof,Limits limits={},Stats* stats=nullptr,
        ApprovedProviderLeases* leases=nullptr,bool acquireLeases=false)
    {
        proof={};Observation before(limits),after(limits);
        bool ok=before.Capture(entry,true) && after.Capture(entry,false) &&
            before.rootKind==after.rootKind && before.used==after.used &&
            before.stamp.epoch==after.stamp.epoch && before.stamp.invalidation==after.stamp.invalidation &&
            IdentityStampValid(before.stamp) && !std::memcmp(before.bytes.data(),after.bytes.data(),before.used);
        if(stats){*stats=before.stats;stats->reads=before.stats.reads>after.stats.reads?before.stats.reads:after.stats.reads;}
        if(!ok)return false;
        Hash hash;if(!hash.Add(before.bytes.data(),before.used) || !hash.Finish(proof.digest) || !IdentityStampValid(before.stamp))return false;
        if(leases && !leases->Accept(before,after,acquireLeases)){proof={};return false;}
        proof.kind=before.rootKind;return true;
    }
}
