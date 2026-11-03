#include "semantic-attribution/native_event_adapter.h"
#include <windows.h>
#include <algorithm>
#include <atomic>
#include <cstring>
#include <memory>
#include <new>

extern "C" std::uint64_t SF_SemanticXsaveMask{};
namespace SemanticAttribution {
namespace {
std::atomic<NativeEventAdapter*> transport{};
template<class T> T issued(const EventReply& r) noexcept {
    if(auto* v=std::get_if<T>(&r.issued)) return *v;
    return {};
}
template<class T> T related(const EventReply& r) noexcept {
    if(auto* v=std::get_if<T>(&r.related)) return *v;
    return {};
}
template<class T> bool valid(T id) noexcept {return id!=T{};}
constexpr std::size_t MaxRows=32,MaxBindings=1024,MaxRoots=32,MaxRecords=2048,MaxNodes=2048,MaxOps=128;
struct Association {
    std::uintptr_t system{};
    SystemBindingId binding{};
    CompileId compile{};
};
struct QueryProvenance {
    std::uintptr_t slot{};
    std::uint32_t ordinal{},group{};
    HashDirection direction{};
};
struct RowState {
    bool used{};
    NativeThreadScalar thread{};
    std::uintptr_t frame{},row{},context{},world{},manager{},owner{},context_scope_frame{},world_setup_context{};
    RowPassId pass{};
    RowIterationId iteration{};
    ReservationId reservation{},context_reservation{},world_reservation{};
    ContextSetupId setup{};
    ContextBirthId context_birth{};
    ContextPublicationId context_publication{};
    WorldBirthId world_birth{};
    WorldPublicationId world_publication{};
    std::array<Association,MaxBindings> snapshot{};
    std::uint32_t snapshot_count{};
};
struct BindingState {
    bool used{},entry_complete{},read_count_seen{},write_count_seen{},hash_pending{};
    std::uintptr_t world{},system{},type{},callback{},registration_frame{},compiler_frame{},compiler_parent{},slot{};
    WorldPublicationId world_publication{};
    RegistrationOpId registration{};
    SystemBindingId binding{};
    CompileOpId compiler{};
    CompileId compile{};
    std::uint16_t ordinal{};
    std::uint32_t flags_one{},flags_two{};
    std::uint8_t disable{},flag_one{},flag_two{};
    std::uint32_t read_count{},write_count{},hash{},component_ordinal{};
    HashDirection direction{};
    std::array<QueryProvenance,42> pending_queries{};
    std::uint32_t pending_query_count{};
};
struct CompiledState {
    bool used{};
    CompileId compile{};
    WorldPublicationId world{};
    std::uintptr_t schema{};
    std::array<QueryProvenance,42> queries{};
    std::uint32_t count{};
};
struct PublishedRow {
    bool used{};
    std::uintptr_t row{},context{},world{};
    ContextPublicationId context_publication{};
    WorldPublicationId world_publication{};
};
struct RootState {
    bool used{};
    NativeThreadScalar thread{};
    std::uintptr_t frame{},scheduler_frame{},manager{},parent_address{},child_address{},parent_descriptor{},child_descriptor{};
    std::uint32_t row_index{};
    RootAttributionId root{};
    DispatchGroupId group{};
    CreateOpId parent_create{},child_create{};
    NodeBirthId parent{},child{};
    TargetEdgeId edge{};
    OriginalSubmitId submit{};
    std::uintptr_t current_system{};
};
struct RecordState {
    bool used{},associated{};
    RootAttributionId root{};
    DispatchGroupId group{};
    DispatchRecordId record{};
    Association association{};
    std::uintptr_t address{},manager{};
};
struct NodeState {
    bool used{};
    NodeBirthId birth{},parent{};
    OriginalSubmitId submit{};
    TargetEdgeId edge{};
    PublicationId publication{};
    std::uintptr_t address{},owner{},target{},root_slot{},payload_low{},payload_high{},native_callback{},native_target{};
    bool native_target_seen{};
    std::uint32_t pending{};
    RootAttributionId root{};
    DispatchGroupId group{};
    CreateOpId create{};
    // Copied at this node's actual M/R payload store. Old children never search
    // a later R/address map to rediscover their system/compiler association.
    std::array<RecordState,kMaxDispatchRecords> records{};
    std::uint32_t record_count{};
};
struct OperationState {
    bool used{};
    NativeThreadScalar thread{};
    std::uintptr_t frame{},owner{},node{},source{},target{},system{},source_entry{},destination{},output_base{},expected_output{};
    std::uint32_t kind{},first_ordinal{},last_ordinal{},index{},stride{},flag_one{},flag_two{};
    bool optional{},last_run{},cached{};
    RegistrationOpId registration{};
    WorldPublicationId world{};
    GateHoldId gate{};
    ClaimId claim{};
    NodeBirthId birth{};
    LinkAttemptId link{};
    MaterializeOpId materializer{};
    PublicationId publication{};
    Association association{};
    DispatchRecordId dispatch{};
    RootAttributionId root{};
    RowIterationId row_iteration{};
    bool callback_entered{};
    bool entry_active{};
    std::uintptr_t materializer_frame{};
    std::uint32_t completed_count{},entry_witnesses{};
};
}
struct NativeEventAdapter::Impl {
    Ledger* ledger{};
    std::uint64_t incarnation{};
    std::uintptr_t image{};
    std::uint32_t image_size{};
    std::span<const Native::ResidentSite> sites;
    SRWLOCK gate=SRWLOCK_INIT;
    std::atomic<bool> poisoned{},published{};
    std::array<RowState,MaxRows> rows{};
    std::array<PublishedRow,256> published_rows{};
    std::array<BindingState,MaxBindings> bindings{};
    std::array<CompiledState,MaxBindings> compiled{};
    std::array<RootState,MaxRoots> roots{};
    std::array<RecordState,MaxRecords> records{};
    std::array<NodeState,MaxNodes> nodes{};
    std::array<OperationState,MaxOps> operations{};
    template<class T,std::size_t N> T* vacant(std::array<T,N>& a) noexcept {
        for(auto& v:a) if(!v.used) {v={};v.used=true;return &v;}
        poisoned.store(true,std::memory_order_release);return nullptr;
    }
    RowState* row(const Native::FrameChain& chain,NativeThreadScalar thread) noexcept {
        auto* f=chain.Find(0x7b40a0);if(!f)return nullptr;
        for(auto& r:rows) if(r.used && r.thread==thread && r.frame==f->entry_rsp)return &r;
        return nullptr;
    }
    RootState* root(const Native::FrameChain& chain,NativeThreadScalar thread) noexcept {
        auto* f=chain.Find(0x6c0ef0);if(!f)return nullptr;
        for(auto& r:roots)if(r.used && r.thread==thread && r.frame==f->entry_rsp)return &r;
        return nullptr;
    }
    OperationState* operation(std::uintptr_t frame,NativeThreadScalar thread,std::uint32_t kind,bool create=false) noexcept {
        for(auto& o:operations)if(o.used && o.frame==frame && o.thread==thread && o.kind==kind)return &o;
        if(!create)return nullptr;
        auto* o=vacant(operations);if(o){o->frame=frame;o->thread=thread;o->kind=kind;}return o;
    }
    NodeState* node(std::uintptr_t address) noexcept {
        for(auto& n:nodes)if(n.used && n.address==address)return &n;
        return nullptr;
    }
    NodeState* node(NodeBirthId birth) noexcept {
        for(auto& n:nodes)if(n.used && n.birth==birth)return &n;
        return nullptr;
    }
    Association association(const RowState& row,std::uintptr_t system) noexcept {
        for(std::uint32_t i=0;i<row.snapshot_count;++i)if(row.snapshot[i].system==system)return row.snapshot[i];
        return {};
    }
    PublishedRow* publication(std::uintptr_t address,bool create=false) noexcept {
        for(auto& r:published_rows)if(r.used && r.row==address)return &r;
        if(!create)return nullptr;
        auto* r=vacant(published_rows);if(r)r->row=address;return r;
    }
    const QueryProvenance* query(CompileId compile,std::uintptr_t slot,std::uint32_t ordinal) const noexcept {
        for(const auto& c:compiled)if(c.used && c.compile==compile) {
            const QueryProvenance* found{};
            for(std::uint32_t i=0;i<c.count;++i)if(c.queries[i].slot==slot && c.queries[i].ordinal==ordinal) {
                if(found)return nullptr;found=&c.queries[i];
            }
            return found;
        }
        return nullptr;
    }
    void forget_world(WorldPublicationId world) noexcept {
        for(auto& b:bindings)if(b.used && b.world_publication==world)b.used=false;
        for(auto& c:compiled)if(c.used && c.world==world)c.used=false;
    }
    void forget_schema(std::uintptr_t schema) noexcept {
        for(auto& c:compiled)if(c.used && c.schema==schema)c.used=false;
        for(auto& b:bindings)if(b.used && b.world+0x928==schema)b.compile={};
    }
};
std::size_t NativeEventAdapter::RequiredStorage() noexcept {return sizeof(NativeEventAdapter)+sizeof(Impl)+alignof(Impl);}
NativeEventAdapter* NativeEventAdapter::Construct(std::span<std::byte> storage,Ledger& ledger,
    std::uint64_t incarnation,std::uintptr_t image,std::uint32_t image_size,
    std::span<const Native::ResidentSite> sites) noexcept {
    if(!incarnation || !image || sites.empty() || storage.size()<RequiredStorage())return nullptr;
    void* memory=storage.data();auto space=storage.size();
    if(!std::align(alignof(NativeEventAdapter),sizeof(NativeEventAdapter),memory,space))return nullptr;
    auto* self=static_cast<NativeEventAdapter*>(memory);
    memory=static_cast<std::byte*>(memory)+sizeof(NativeEventAdapter);space-=sizeof(NativeEventAdapter);
    if(!std::align(alignof(Impl),sizeof(Impl),memory,space))return nullptr;
    auto* impl=new(memory)Impl;impl->ledger=&ledger;impl->incarnation=incarnation;impl->image=image;
    impl->image_size=image_size;impl->sites=sites;
    return new(self)NativeEventAdapter(impl);
}
NativeEventAdapter::~NativeEventAdapter(){Close();impl_->~Impl();}
void NativeEventAdapter::Close() noexcept {
    PoisonAndClose(CloseReason::Shutdown);
}
void NativeEventAdapter::PoisonAndClose(CloseReason reason) noexcept {
    impl_->poisoned.store(true,std::memory_order_release);
    AcquireSRWLockExclusive(&impl_->gate);
    CloseLocked(reason);
    ReleaseSRWLockExclusive(&impl_->gate);
}
void NativeEventAdapter::CloseLocked(CloseReason reason) noexcept {
    // A previously loaded transport pointer remains resident. Draining this
    // gate serializes every ledger mutation, not the outside-gate stack walk.
    impl_->ledger->CloseAll(reason);
    impl_->published.store(false,std::memory_order_release);
    auto* expected=this;transport.compare_exchange_strong(expected,nullptr,std::memory_order_acq_rel);
}
bool NativeEventAdapter::PublishTransport() noexcept {
    AcquireSRWLockExclusive(&impl_->gate);
    if(impl_->poisoned.load(std::memory_order_acquire)) {
        ReleaseSRWLockExclusive(&impl_->gate);return false;
    }
    NativeEventAdapter* expected=nullptr;
    const bool inserted=transport.compare_exchange_strong(expected,this,std::memory_order_acq_rel);
    if(inserted)impl_->published.store(true,std::memory_order_release);
    ReleaseSRWLockExclusive(&impl_->gate);return inserted;
}
EventReply NativeEventAdapter::Send(NativeSite site,NativeFrameScalar frame,const Event& e) noexcept {
    return impl_->ledger->Observe(NativeReceipt(site,GetCurrentThreadId(),frame,impl_->incarnation,
        Native::ArchivedProfileIdentity),e);
}
void DispatchNativeAttribution(Native::SavedMachine* f,const void* xstate) noexcept {
    auto* adapter=transport.load(std::memory_order_acquire);
    if(adapter && f)adapter->Capture(*f,xstate);
}
void NativeEventAdapter::Capture(Native::SavedMachine& f,const void* xstate) noexcept {
    auto& p=*impl_;
    if(!p.published.load(std::memory_order_acquire))return;
    if(p.poisoned.load(std::memory_order_acquire)){PoisonAndClose(CloseReason::Explicit);return;}
    const Native::ResidentSite* captured{};Native::CapturePhase phase{};
    for(const auto& r:p.sites) {
        if(r.before_return && f.relay_return==r.relay+r.before_return){captured=&r;phase=Native::CapturePhase::Before;break;}
        if(r.after_return && f.relay_return==r.relay+r.after_return){captured=&r;phase=Native::CapturePhase::After;break;}
    }
    if(!captured || f.site_return!=p.image+captured->site->rva+5)return;
    Native::FrameChain chain{};
    if(!Native::CaptureOriginalFrames(*captured->site,phase,f,p.image,p.image_size,chain)) {
        PoisonAndClose(CloseReason::Explicit);return;
    }
    // The native fifth argument is exactly a DWORD. Its bounded stack/SEH read
    // belongs with frame capture, before any host gate or ledger mutation.
    std::uint32_t original_row_index{};
    const bool row_index_read=captured->site->rva!=0x622ad0 ||
        Native::ReadStackU32(f.NativeRsp()+0x28,original_row_index);
    std::uint64_t payload_words[2]{};
    if(captured->site->rva==0x5cec7b && xstate)
        std::memcpy(payload_words,static_cast<const std::byte*>(xstate)+160,sizeof(payload_words));
    // Ordinary worker contention serializes bounded HOST work. Treating a
    // contended lock as native reentry would quarantine every parallel queue.
    // The protected code allocates nothing and calls no engine/foreign callback;
    // native operations and exception-prone stack walking are outside this gate.
    AcquireSRWLockExclusive(&p.gate);
    if(p.poisoned.load(std::memory_order_acquire) || !p.published.load(std::memory_order_acquire)) {
        CloseLocked(CloseReason::Explicit);ReleaseSRWLockExclusive(&p.gate);return;
    }
    const auto thread=NativeThreadScalar(GetCurrentThreadId());
    auto* row=p.row(chain,thread);auto* root=p.root(chain,thread);
    const auto frame=chain.frames[0].entry_rsp;
    const auto site=captured->site->rva;
    auto send=[&](NativeSite label,const auto& event,NativeFrameScalar origin=0) {
        return Send(label,origin?origin:frame,Event{event});
    };
    auto mutate=[&](std::uintptr_t schema,SchemaMutationKind kind,NativeSite label) {
        send(label,SchemaMutate{std::nullopt,schema,kind});
        p.forget_schema(schema);
    };
    auto forget_publications=[&](std::uintptr_t address,unsigned kind) {
        for(auto& pub:p.published_rows)if(pub.used &&
            (kind==0?pub.context==address:kind==1?pub.world==address:kind==2?pub.world+0x928==address:pub.world+0xe15dc0==address)) {
            p.forget_world(pub.world_publication);pub.world_publication={};pub.world=0;
            if(kind==0)pub.used=false;
        }
    };
    switch(site) {
    case 0x5d0cb0:
        if(auto* work=p.operation(frame,thread,3);work && work->node==f.rbx && f.r12==p.image+kWorkloadCallback && valid(work->claim)) {
            if(!work->callback_entered) {
                const auto reply=send(NativeSite::WorkCallbackBegin,WorkCallbackBegin{work->claim,work->birth,kWorkloadCallback});
                work->callback_entered=reply.status==EventStatus::Accepted;
            }
            work->index=static_cast<std::uint32_t>(f.rsi);
        }
        break;
    case 0x643ee7: {
        auto* op=p.operation(frame,thread,6,true);if(!op)break;
        *op={};op->used=true;op->kind=6;op->frame=frame;op->thread=thread;op->system=f.rbx;op->expected_output=f.rbp+0x108;
        const auto parent_return=chain.frames[0].return_pc;
        if(parent_return==p.image+0x6433e4 && root) {
            for(const auto& record:p.records)if(record.used && record.associated && record.root==root->root &&
                record.address==f.rdi && record.association.system==f.rbx) {
                op->root=root->root;op->association=record.association;op->dispatch=record.record;break;
            }
        } else if(parent_return==p.image+0x683859) {
            auto* workload=chain.Find(0x683800);auto* worker=chain.Find(0x5d0940);
            if(!workload || workload->return_pc!=p.image+0x5d0cbc || !worker)break;
            auto* work=p.operation(worker->entry_rsp,thread,3);
            if(!work || !work->callback_entered || !valid(work->claim))break;
            auto* node=p.node(work->birth);if(!node || node->address!=work->node)break;
            if(work->index>=node->record_count)break;
            const auto& record=node->records[work->index];
            if(record.address!=f.rdi || record.association.system!=f.rbx)break;
            op->claim=work->claim;op->birth=node->birth;op->publication=node->publication;
            op->association=record.association;op->dispatch=record.record;
        }break;}
    case 0x622ad0: {
        auto* materializer=chain.Find(0x643c80);if(!materializer || chain.frames[0].return_pc!=p.image+0x643f72)break;
        auto* op=p.operation(materializer->entry_rsp,thread,6);if(!op)break;
        op->materializer={};op->entry_active=false;op->completed_count=0;
        if(f.rcx!=op->expected_output || f.rdx!=op->system+0x6d8 || !row_index_read)break;
        op->index=original_row_index;
        if(!valid(op->association.compile) || !valid(op->dispatch))break;
        const bool queued=valid(op->claim);
        std::optional<NodeBirthId> parent,child;
        if(queued)if(auto* node=p.node(op->birth)){if(valid(node->parent))parent=node->parent;child=node->birth;}
        op->materializer_frame=frame;
        op->materializer=issued<MaterializeOpId>(send(NativeSite::MaterializeBegin,
            MaterializeBegin{queued?std::variant<RootAttributionId,ClaimId>(op->claim):std::variant<RootAttributionId,ClaimId>(op->root),
                queued?std::optional(op->publication):std::nullopt,op->association.compile,op->association.binding,
                op->dispatch,parent,child}));break;}
    case 0x622b54: {
        auto* materializer=chain.Find(0x643c80);if(!materializer)break;
        auto* op=p.operation(materializer->entry_rsp,thread,6);if(!op || !valid(op->materializer))break;
        // Only the current entry is replaced; the owned operation is never
        // reacquired after a source cursor or native frame changes.
        op->entry_active=false;op->entry_witnesses=0;op->source_entry=f.r9;op->destination=f.r8-0xa;
        if(frame!=op->materializer_frame || op->completed_count>=42 ||
           op->source_entry!=op->system+0x6dc+op->completed_count*0xc ||
           op->destination!=op->expected_output+op->completed_count*0x10 || f.rbp!=op->index)break;
        op->first_ordinal=static_cast<std::uint16_t>(f.r10);
        op->flag_one=0;op->flag_two=0;op->optional=false;
        op->entry_active=true;op->entry_witnesses=1;break;}
    case 0x622ba6:case 0x622bb2:case 0x622bc2:case 0x622bcb:case 0x622bd4: {
        auto* materializer=chain.Find(0x643c80);if(!materializer)break;
        auto* op=p.operation(materializer->entry_rsp,thread,6);if(!op || !valid(op->materializer) || !op->entry_active)break;
        if(site==0x622ba6){op->optional=(f.flags&0x40)==0;op->entry_witnesses|=2;}
        else if(site==0x622bb2) {
            if(f.r9!=op->source_entry || f.r8-0xa!=op->destination || f.rbp!=op->index){op->entry_active=false;break;}
            op->output_base=f.rcx;op->stride=static_cast<std::uint16_t>(f.rdx);
            op->entry_witnesses|=4;
        } else if(site==0x622bc2) {
            if(f.r9!=op->source_entry || f.r8-0xa!=op->destination){op->entry_active=false;break;}
            op->last_ordinal=static_cast<std::uint16_t>(f.rax);op->entry_witnesses|=8;
        } else if(site==0x622bcb) {
            if(phase==Native::CapturePhase::Before){op->flag_one=static_cast<std::uint8_t>(f.rax);op->entry_witnesses|=16;}
            else {op->flag_two=static_cast<std::uint8_t>(f.rax);op->entry_witnesses|=32;}
        } else {
            op->entry_active=false;
            if(f.r9-0xc!=op->source_entry || f.r8-0xa!=op->destination || op->entry_witnesses!=63 ||
               (!op->output_base && !op->optional))break;
            const auto* query=p.query(op->association.compile,op->source_entry,op->first_ordinal);
            if(!query)break;
            const auto reply=send(NativeSite::MaterializeOutput,MaterializeEntryCaptured{op->materializer,op->source_entry,
                op->first_ordinal,op->last_ordinal,query->direction,query->group,op->output_base,op->stride,
                op->index,op->destination},op->materializer_frame);
            if(reply.status==EventStatus::Accepted) {
                ++op->completed_count;
            }
        }break;}
    case 0x643f72:
        if(auto* op=p.operation(frame,thread,6)) {
            // A complete per-entry store is not a complete native invocation.
            // Publish only after the unchanged622AD0 CALL returned normally.
            if(valid(op->materializer) && f.rax<=UINT32_MAX && f.r11<=UINT32_MAX)
                send(NativeSite::MaterializeComplete,MaterializeComplete{op->materializer,
                    static_cast<std::uint32_t>(f.rax),static_cast<std::uint32_t>(f.r11)},op->materializer_frame);
            op->used=false;
        }break;
    case 0x643053:case 0x6431a2:
        if(root && row && valid(root->root) && f.r10==root->manager) {
            const auto association=p.association(*row,f.r13);
            if(!valid(association.binding) || !valid(association.compile))break;
            const auto address=site==0x643053?f.rcx:f.rcx-0x50;
            auto* record=p.vacant(p.records);if(!record)break;
            record->root=root->root;record->association=association;record->address=address;record->manager=f.r10;
            record->record=issued<DispatchRecordId>(send(NativeSite::DispatchRecordCaptured,
                DispatchRecordCaptured{root->root,association.compile,association.binding,f.r13,address,site==0x643053?0U:1U}));
            if(auto* scheduler=chain.Find(0x642a60))root->scheduler_frame=scheduler->entry_rsp;
        }break;
    case 0x643202:
        if(root && valid(root->root)) {
            root->group={};root->parent_create={};root->child_create={};root->parent={};root->child={};root->edge={};root->submit={};
            root->parent_address=0;root->child_address=0;root->current_system=f.r13;
            root->group=issued<DispatchGroupId>(send(NativeSite::DispatchGroupBegin,
                DispatchGroupBegin{root->root,f.r13,frame}));
            for(auto& record:p.records)if(record.used && !record.associated && record.root==root->root && record.association.system==f.r13) {
                const auto reply=send(NativeSite::DispatchRecordAssociated,
                    DispatchRecordAssociated{root->group,record.record,record.manager,record.address,f.r13});
                record.associated=reply.status==EventStatus::Committed;record.group=root->group;
            }
        }break;
    case 0x643251:
        if(root){root->parent_descriptor=f.rbp-0x20;root->parent_create=issued<CreateOpId>(send(NativeSite::ParentCreateBegin,
            ParentCreateBegin{root->root,root->group,root->parent_descriptor,kApprovedParentCreator,0}));}
        break;
    case 0x6432c4:
        if(root){root->edge={};root->edge=issued<TargetEdgeId>(send(NativeSite::DescriptorTargetAssign,
            DescriptorTargetAssigned{root->group,f.rcx?std::optional(root->parent):std::nullopt,f.rcx}));}
        break;
    case 0x643337:
        if(root){root->child_descriptor=f.rdx;root->child_create=issued<CreateOpId>(send(NativeSite::ChildCreateBegin,
            NodeCreateBegin{root->root,root->group,root->parent,f.rdx,kWorkloadCallback,root->edge}));}
        break;
    case 0x5cebff:
        if(root && f.rax && chain.Find(0x5d1ce0)) {
            bool parent_path=false,child_path=false;
            for(std::uint32_t i=0;i<chain.count;++i){parent_path|=chain.frames[i].return_pc==p.image+0x643283;child_path|=chain.frames[i].return_pc==p.image+0x64334d;}
            if(!parent_path && !child_path)break;
            auto* node=p.node(f.rax);if(node)*node={};else node=p.vacant(p.nodes);if(!node)break;
            node->used=true;node->address=f.rax;node->owner=f.rsi;node->root=root->root;node->group=root->group;
            node->create=parent_path?root->parent_create:root->child_create;
            const auto reply=send(NativeSite::NodeAllocationReturn,NodeBirthCaptured{node->create,f.rax,f.rsi,true});
            node->birth=issued<NodeBirthId>(reply);node->submit=related<OriginalSubmitId>(reply);
            if(parent_path){root->parent=node->birth;root->parent_address=node->address;}
            else {root->child=node->birth;root->submit=node->submit;root->child_address=node->address;node->parent=root->parent;node->edge=root->edge;node->target=root->parent_address;}
        }break;
    case 0x5cec6f:
        if(root)if(auto* node=p.node(f.rbx);node && (node->birth==root->child || node->birth==root->parent))
            node->native_callback=f.rax;
        break;
    case 0x5cec7b:
        if(root)if(auto* node=p.node(f.rbx);node && node->birth==root->child && xstate) {
            const auto& words=payload_words;
            node->payload_low=words[0];node->payload_high=words[1];
            node->record_count=0;
            const auto count=static_cast<std::uint32_t>(f.rax);
            if(count<=node->records.size()) {
                for(std::uint32_t i=0;i<count;++i)for(const auto& record:p.records)
                    if(record.used && record.associated && record.root==node->root && record.group==node->group &&
                       record.address==words[1]+std::uintptr_t(i)*0x38 && record.manager==words[0]) {
                        node->records[node->record_count++]=record;break;
                    }
                if(node->record_count!=count)node->record_count=0;
            }
            if(words[0]==root->manager && count && node->record_count==count) {
                ChildPayloadCaptured payload{node->birth,node->owner,words[0],words[1]};
                payload.dispatch_record_count=count;
                for(std::uint32_t i=0;i<count;++i)payload.dispatch_records[i]=node->records[i].record;
                send(NativeSite::ChildPayloadCopy,payload);
            }
        }break;
    case 0x5cec9a:
        if(root)if(auto* node=p.node(f.rbx);node &&
            ((node->birth==root->child && f.rdi==root->child_descriptor) ||
             (node->birth==root->parent && f.rdi==root->parent_descriptor))) {
            node->native_target=f.rax;node->native_target_seen=true;
        }
        break;
    case 0x5ceca5:
        if(root)if(auto* node=p.node(f.rbx);node && (node->birth==root->parent || node->birth==root->child)) {
            const bool child=node->birth==root->child;
            if(node->native_callback!=p.image+(child?kWorkloadCallback:kApprovedParentCreator) ||
               !node->native_target_seen || node->native_target!=node->target || (child && !node->record_count))break;
            if(child)send(NativeSite::ChildTargetStore,ChildTargetEdgeCaptured{node->create,node->edge,node->parent,node->birth,
                node->target,node->native_target,node->native_target});
            node->pending=static_cast<std::uint32_t>(f.r14);send(NativeSite::NodePayloadComplete,NodePayloadCompleted{node->birth,true});
        }break;
    case 0x643283:
        if(root)send(NativeSite::ParentBirthReturn,ParentCreatorReturned{root->parent_create,f.rax,kApprovedParentCreator,0});
        break;
    case 0x64334d:
        if(root)send(NativeSite::ChildCreatorReturn,ChildCreatorReturned{root->child_create,root->child,f.rax});
        break;
    case 0x64337a:
        if(root)if(auto* node=p.node(root->child);node && f.rcx<kRootKickSlotLimit) {
            node->root_slot=f.rbp+0x130+f.rcx*8;
            send(NativeSite::RootKickAssignment,RootKickAssigned{root->root,root->group,node->submit,node->birth,
                node->root_slot,f.rax,kWorkloadCallback,f.r15,node->pending,static_cast<std::uint32_t>(f.rcx),true});
        }break;
    case 0x5cfdee: {
        auto* op=p.operation(frame,thread,4,true);if(!op)break;
        *op={};op->used=true;op->frame=frame;op->thread=thread;op->kind=4;op->target=f.rdi;
        if(chain.frames[0].return_pc==p.image+0x6434f8 && root) {
            for(auto& n:p.nodes)if(n.used && n.root==root->root && n.root_slot==f.rbx && n.address==f.rdi) {
                op->birth=n.birth;op->source=n.root_slot;op->node=n.address;break;
            }
        } else if(chain.frames[0].return_pc==p.image+0x5d0d6a) {
            if(auto* completion=chain.Find(0x5d0d30))if(auto* source=p.operation(completion->entry_rsp,thread,5)) {
                op->claim=source->claim;op->birth=source->birth;op->source=source->source;op->cached=source->cached;
                op->gate=source->gate;op->last_run=source->last_run;
                if(source->target!=f.rdi)op->birth={};
            }
        }break;}
    case 0x5cfe4a: {
        auto* op=p.operation(frame,thread,4);
        if(!op || !valid(op->birth) || f.rdi!=op->target) {
            send(NativeSite::LinkTargetRetire,LinkTargetRetire{f.rdi,f.rsi});break;
        }
        auto* source=p.node(op->birth);
        if(!source){send(NativeSite::LinkTargetRetire,LinkTargetRetire{f.rdi,f.rsi});break;}
        const auto owner=f.rsi;const auto gate=owner+0x148;
        if(valid(op->claim)) {
            const auto mode=op->cached?LinkMode::CallerHeldGate:LinkMode::AcquiredGate;
            op->link=issued<LinkAttemptId>(send(NativeSite::DelayedCompletionTarget,
                DelayedLinkAttemptBegin{op->claim,source->submit,source->birth,source->edge,source->parent,
                    f.rdi,owner,mode,gate,op->cached?std::optional(op->gate):std::nullopt,
                    op->cached?CompletionOrigin::CachedReturn:CompletionOrigin::WorkerReturn,op->last_run,
                    op->source,f.rdi}));
            send(NativeSite::DelayedLinkRequestEntered,DelayedLinkRequestEntered{op->link,op->source,f.rdi});
        } else {
            op->link=issued<LinkAttemptId>(send(NativeSite::RootLinkAttemptBegin,
                RootLinkAttemptBegin{source->submit,op->source,f.rdi,owner,LinkMode::AcquiredGate,gate,std::nullopt}));
        }
        op->owner=owner;
        const auto checked=send(NativeSite::LinkPrevalidate,LinkPrevalidated{op->link,f.rdi,owner,gate,f.rbx==gate});
        if(checked.status!=EventStatus::Pending)send(NativeSite::LinkTargetRetire,LinkTargetRetire{f.rdi,owner});
        break;}
    case 0x5cfe74:
        if(auto* op=p.operation(frame,thread,4);op && valid(op->link) && op->owner==f.rsi && op->target==f.rdi) {
            const auto pub=issued<PublicationId>(send(NativeSite::LinkCommitted,
                LinkCommitted{op->link,f.rdi,f.rsi,f.rsi+0x148,true,true,true}));
            if(auto* source=p.node(op->birth)) {
                auto* target=valid(op->claim)?p.node(source->parent):source;
                if(target && target->address==f.rdi)target->publication=pub;
            }
        }break;
    case 0x6434f8:
        if(root)for(auto& n:p.nodes)if(n.used && n.root==root->root && n.root_slot==f.rbx-8) {
            send(NativeSite::RootSubmitTailReturn,RootSubmitTailReturned{n.submit,std::nullopt});break;
        }break;
    case 0x5d1c58: {
        auto* op=p.operation(frame,thread,2,true);if(!op)break;
        *op={};op->used=true;op->frame=frame;op->thread=thread;op->kind=2;op->owner=f.rsi;op->node=f.rdx;op->cached=true;
        if(f.rbx==f.rsi+0x148)op->gate=issued<GateHoldId>(send(NativeSite::GateHoldBegin,GateHoldBegin{f.rsi,f.rsi+0x148}));
        send(NativeSite::CachedSourceReserved,CachedSourceReserved{op->gate,f.rdx});break;}
    case 0x5d1c7b:
        if(auto* op=p.operation(frame,thread,2);op && op->node==f.rcx) {
            op->claim=issued<ClaimId>(send(NativeSite::CachedClaim,QueueClaimCaptured{ClaimKind::Cached,op->gate,f.rcx,true}));
            if(auto* node=p.node(f.rcx))op->birth=node->birth;
        }break;
    case 0x5d1be6:
        if(auto* hold=chain.Find(0x5d1c10))if(auto* op=p.operation(hold->entry_rsp,thread,2);op && op->owner==f.rbx) {
            op->claim={};op->birth={};op->node=f.rax;op->cached=false;
            op->claim=issued<ClaimId>(send(NativeSite::FreshClaim,QueueClaimCaptured{ClaimKind::Fresh,op->gate,f.rax,true},hold->entry_rsp));
            if(auto* node=p.node(f.rax))op->birth=node->birth;
        }break;
    case 0x5d1cba:
        if(auto* op=p.operation(frame,thread,2);op && op->node==f.rax && valid(op->claim)) {
            send(NativeSite::ClaimTransfer,QueueClaimTransfer{op->claim,op->birth});
            if(auto* worker=chain.Find(0x5d0940))if(auto* work=p.operation(worker->entry_rsp,thread,3,true)) {
                *work=*op;work->frame=worker->entry_rsp;work->kind=3;work->cached=false;work->gate={};
            }
        }break;
    case 0x5d1cc4:
        if(auto* op=p.operation(frame,thread,2)) {send(NativeSite::GateHoldRelease,GateHoldRelease{op->gate,op->owner});op->used=false;}
        break;
    case 0x5d0cf1:
        if(auto* op=p.operation(frame,thread,3);op && op->node==f.rcx)
            send(NativeSite::CompletionBegin,CompletionBegin{op->claim,op->birth});
        break;
    case 0x5d0d3f: {
        OperationState* source{};
        if(auto* hold=chain.Find(0x5d1c10))source=p.operation(hold->entry_rsp,thread,2);
        else if(auto* worker=chain.Find(0x5d0940))source=p.operation(worker->entry_rsp,thread,3);
        auto* completion=p.operation(frame,thread,5,true);if(!completion)break;
        *completion={};completion->used=true;completion->frame=frame;completion->thread=thread;completion->kind=5;
        if(source && source->node==f.rsi) {
            completion->claim=source->claim;completion->birth=source->birth;completion->source=f.rsi;
            completion->cached=source->cached;completion->gate=source->gate;
            if(source->cached)send(NativeSite::CompletionBegin,CompletionBegin{source->claim,source->birth});
        }break;}
    case 0x5d0d56:
        if(auto* op=p.operation(frame,thread,5);op && op->source==f.rsi){op->target=f.rcx;op->last_run=f.rax==1;}
        break;
    case 0x5d0d6a:
        if(auto* op=p.operation(frame,thread,5);op && op->source==f.rsi && valid(op->birth)) {
            // The actual source owner was loaded by native code after the
            // handoff call. Keep the original claim until the selected outer
            // completion return; do not look up or adopt another source node.
            if(auto* node=p.node(op->birth);node && node->owner==f.rbx)op->owner=f.rbx;
            else p.poisoned.store(true);
        }
        break;
    case 0x5d0cfb:
        if(auto* op=p.operation(frame,thread,3);op && op->node==f.rbx){send(NativeSite::CompletionRelease,CompletionRelease{op->claim,op->birth});op->used=false;}
        break;
    case 0x5d1cac:
        if(auto* op=p.operation(frame,thread,2);op && valid(op->claim)) {
            send(NativeSite::CachedClaimAbandon,CachedClaimAbandon{op->claim});op->claim={};op->birth={};
        }break;
    case 0x5cece1:
        send(NativeSite::RejectedNodeFree,RejectedNodeFree{f.rbx});
        if(auto* node=p.node(f.rbx))node->used=false;
        break;
    case 0x5d57a6:
        send(NativeSite::NodeRetire,NodeRetire{std::nullopt,f.rdx});
        if(auto* node=p.node(f.rdx))node->used=false;
        break;
    case 0x6778e0: {
        auto* op=p.operation(frame,thread,1,true);if(!op)break;*op={};op->used=true;op->thread=thread;op->frame=frame;op->kind=1;
        auto* setup=chain.Find(0x7a6590);auto* native_batch=chain.Find(0x22e680);
        auto* batch=setup?p.operation(setup->entry_rsp,thread,7):nullptr;
        bool known_type=false;
        for(const auto& definition:Native::ArchivedServerProfile().registration_catalog)
            if(p.image+definition.descriptor_rva==f.rdx){known_type=true;break;}
        if(batch && native_batch && native_batch->return_pc==p.image+0x7a701b && f.rcx==batch->owner && known_type) {
            op->owner=f.rcx;op->source=f.rdx;op->index=static_cast<std::uint8_t>(f.r8);op->world=batch->world;op->expected_output=batch->frame;
            op->registration=issued<RegistrationOpId>(send(NativeSite::RegistrationBegin,
                RegistrationBegin{batch->world,f.rcx,f.rdx,static_cast<std::uint8_t>(f.r8)}));
        }break;}
    case 0x7a6eab:
        if(row && f.rdi==row->context && f.rax==row->world && valid(row->world_publication) &&
           chain.frames[0].return_pc==p.image+0x7b429c) {
            if(auto* batch=p.operation(frame,thread,7,true)) {
                *batch={};batch->used=true;batch->frame=frame;batch->thread=thread;batch->kind=7;
                batch->world=row->world_publication;batch->owner=f.rax;batch->source=f.rdi;
            }
        }break;
    case 0x677ac7:
        send(NativeSite::RegistrationRetire,RegistrationRetire{f.rbx});
        if(auto* op=p.operation(frame,thread,1))op->system=f.rbx;
        for(auto& b:p.bindings)if(b.used && b.system==f.rbx) {
            for(auto& c:p.compiled)if(c.used && c.compile==b.compile)c.used=false;
            b.used=false;
        }
        break;
    case 0x677ad9:
        if(auto* op=p.operation(frame,thread,1);op && op->source==f.r15 && op->system==f.rbx && op->owner==f.rsi)
            op->entry_active=true;
        break;
    case 0x677b20: {
        if(auto* setup=chain.Find(0x7a6590))if(auto* batch=p.operation(setup->entry_rsp,thread,7);batch && batch->owner==f.rsi)
            batch->index=static_cast<std::uint16_t>(f.rdx);
        auto* op=p.operation(frame,thread,1);if(!op || !valid(op->registration) || !op->entry_active || !op->callback_entered ||
            f.r15!=op->source || f.rsi!=op->owner || f.rbx!=op->system)break;
        bool tuple_matches=false;
        for(const auto& d:Native::ArchivedServerProfile().registration_catalog)
            if(p.image+d.descriptor_rva==f.r15 && op->output_base==(d.callback_rva?p.image+d.callback_rva:0) &&
               static_cast<std::uint32_t>(f.rcx)==(d.flags_one|(op->index?2U:0U)) && static_cast<std::uint32_t>(f.rax)==d.flags_two) {
                tuple_matches=true;break;
            }
        if(!tuple_matches)break;
        auto* b=p.vacant(p.bindings);if(!b)break;
        b->world=op->owner;b->world_publication=op->world;b->system=f.rbx;b->type=f.r15;
        // The callback is captured at its actual native store, below. Never load T or S here.
        b->callback=op->output_base;b->registration_frame=frame;b->registration=op->registration;
        b->flags_one=static_cast<std::uint32_t>(f.rcx);b->flags_two=static_cast<std::uint32_t>(f.rax);
        b->ordinal=static_cast<std::uint16_t>(f.rdx);b->disable=static_cast<std::uint8_t>(op->index);
        b->binding=issued<SystemBindingId>(send(NativeSite::RegistrationTupleStore,
            RegistrationStore{b->registration,b->system,b->type,b->callback,b->disable,b->flags_one,b->flags_two,b->ordinal}));
        op->used=false;break;}
    case 0x677ae8:
        if(auto* op=p.operation(frame,thread,1);op && op->entry_active && op->source==f.r15 && op->system==f.rbx && op->owner==f.rsi) {
            op->output_base=f.rax;op->callback_entered=true;
        }
        break;
    case 0x23299b:
        // Completion is dispatched only by the actual normal batch post, not
        // by a future predicted return or a constructor-ready flag.
        if(auto* setup=chain.Find(0x7a6590))if(auto* batch=p.operation(setup->entry_rsp,thread,7);batch &&
            chain.frames[0].return_pc==p.image+0x7a701b && f.rsi==batch->owner) {
            for(auto& b:p.bindings)if(b.used && b.world_publication==batch->world && !b.entry_complete) {
                b.entry_complete=true;
            }
            send(NativeSite::WorldRegistrationBatchComplete,
                WorldRegistrationBatchComplete{batch->world,batch->owner,static_cast<std::uint16_t>(batch->index),true});
            batch->used=false;
        }break;
    case 0x6ba6cd:
        if(row && f.rax==frame && f.r14==row->world && f.rdi==row->world+0x928 &&
           chain.frames[0].return_pc==p.image+0x66a78a && chain.Find(0x66a770) &&
           chain.Find(0x66a770)->return_pc==p.image+0x7b304c && chain.Find(0x7b2940) &&
           chain.Find(0x7b2940)->entry_rsp==row->context_scope_frame) {
            if(auto* compiler=p.operation(frame,thread,8,true)) {
                *compiler={};compiler->used=true;compiler->kind=8;compiler->thread=thread;compiler->frame=frame;
                compiler->world=row->world_publication;compiler->owner=f.r14;compiler->source=f.rdi;
                compiler->row_iteration=row->iteration;if(root)compiler->root=root->root;
            }
        }break;
    case 0x622400: {
        for(const auto& b:p.bindings)if(b.used && b.world==f.rcx-0x928 && valid(b.compiler))p.poisoned.store(true);
        send(NativeSite::CompileAssociationsRetire,CompileAssociationsRetire{f.rcx});
        p.forget_schema(f.rcx);
        auto* parent=chain.Find(0x6ba690);auto* compiler=parent?p.operation(parent->entry_rsp,thread,8):nullptr;
        if(!p.poisoned.load() && compiler && valid(compiler->world) && f.rcx==compiler->source && f.rdx==compiler->owner+0xe15dc0) {
            // Close all old compiler associations BEFORE this actual compiler
            // writes any S. These pending children belong to this exact row.
            for(auto& b:p.bindings)if(b.used && b.entry_complete && b.world_publication==compiler->world) {
                b.compile={};b.compiler={};b.read_count_seen=false;b.write_count_seen=false;b.hash_pending=false;b.compiler_frame=frame;b.pending_query_count=0;
                b.compiler_parent=parent->entry_rsp;
                b.compiler=issued<CompileOpId>(send(NativeSite::CompileBegin,
                    CompileBegin{compiler->world,b.binding,
                        valid(compiler->root)?std::variant<RowIterationId,RootAttributionId>(compiler->root):std::variant<RowIterationId,RootAttributionId>(compiler->row_iteration),
                        compiler->source,std::nullopt,std::nullopt},frame));
            }
        }break;}
    case 0x622690:case 0x622751:
        for(auto& b:p.bindings)if(b.used && b.compiler_frame==frame && b.system==f.rbx-0x8d0 && valid(b.compiler)) {
            b.hash_pending=false;b.slot=f.r8+4;b.hash=static_cast<std::uint32_t>(f.rax);
            b.component_ordinal=static_cast<std::uint16_t>(f.r9);b.direction=site==0x622690?HashDirection::Read:HashDirection::Write;
            b.hash_pending=true;break;
        }break;
    case 0x622763:
        for(auto& b:p.bindings)if(b.used && b.compiler_frame==frame && b.hash_pending && b.slot==f.r8+4)
            b.flag_one=static_cast<std::uint8_t>(f.rax);
        break;
    case 0x6226aa:case 0x622771:
        if(site==0x622771 && phase!=Native::CapturePhase::After)break;
        for(auto& b:p.bindings)if(b.used && b.compiler_frame==frame && b.hash_pending && b.slot==f.r8+4 &&
                                  b.component_ordinal==static_cast<std::uint16_t>(f.r9)) {
            if(site==0x6226aa)b.flag_one=static_cast<std::uint8_t>(f.rax);
            b.flag_two=static_cast<std::uint8_t>(f.rax);
            // group is the actual native READ/WRITE loop branch. flags copies
            // native byte0 into bits0..7 and byte1 into8..15; upper16 are zero.
            const auto group=b.direction==HashDirection::Read?0U:1U;
            const auto reply=send(b.direction==HashDirection::Read?NativeSite::CompileReadHash:NativeSite::CompileWriteHash,
                CompileHashCaptured{b.compiler,b.direction,b.hash,b.slot,b.component_ordinal,
                    group,std::uint32_t(b.flag_one)|(std::uint32_t(b.flag_two)<<8),
                    static_cast<std::uint8_t>(f.r10)!=0},frame);
            if(reply.status==EventStatus::Accepted && b.pending_query_count<b.pending_queries.size())
                b.pending_queries[b.pending_query_count++]={b.slot,b.component_ordinal,group,b.direction};
            b.hash_pending=false;break;
        }break;
    case 0x6226c8:case 0x622793:
        for(auto& b:p.bindings)if(b.used && b.compiler_frame==frame && b.system==f.rbx-0x8d0 && valid(b.compiler)) {
            const auto count=static_cast<std::uint16_t>(f.r14);
            const auto direction=site==0x6226c8?HashDirection::Read:HashDirection::Write;
            if(direction==HashDirection::Read){b.read_count=count;b.read_count_seen=true;}else{b.write_count=count;b.write_count_seen=true;}
            send(direction==HashDirection::Read?NativeSite::CompileReadCount:NativeSite::CompileWriteCount,
                CompileCountCaptured{b.compiler,direction,count});break;
        }break;
    case 0x6ba8a0:
        if(row)for(auto& b:p.bindings)if(b.used && b.compiler_parent==frame && b.world_publication==row->world_publication && valid(b.compiler) && b.read_count_seen && b.write_count_seen && !b.hash_pending && b.pending_query_count==b.read_count+b.write_count) {
            b.compile=issued<CompileId>(send(NativeSite::CompileComplete,CompileComplete{b.compiler,b.read_count,b.write_count,true},b.compiler_frame));
            if(valid(b.compile)) {
                if(auto* c=p.vacant(p.compiled)) {
                    c->compile=b.compile;c->world=b.world_publication;c->schema=b.world+0x928;
                    c->queries=b.pending_queries;c->count=b.pending_query_count;
                }
                bool replaced=false;
                for(std::uint32_t i=0;i<row->snapshot_count;++i)if(row->snapshot[i].system==b.system) {
                    row->snapshot[i]={b.system,b.binding,b.compile};replaced=true;break;
                }
                if(!replaced && row->snapshot_count<row->snapshot.size())row->snapshot[row->snapshot_count++]={b.system,b.binding,b.compile};
            }
            b.compiler={};
        }
        if(auto* compiler=p.operation(frame,thread,8))compiler->used=false;
        break;
    case 0x7b2968:
        if(row && f.r14==row->context && chain.frames[0].return_pc==p.image+0x7b41c9)
            row->context_scope_frame=frame;
        break;
    case 0x6c0efa:
        if(row && valid(row->context_publication) && f.rcx==row->world && valid(row->world_publication) &&
           chain.Find(0x7b2940) && chain.Find(0x7b2940)->entry_rsp==row->context_scope_frame &&
           (chain.frames[0].return_pc==p.image+0x7b35a9 || chain.frames[0].return_pc==p.image+0x7b365b)) {
            if(root){for(auto& record:p.records)if(record.used && record.root==root->root)record.used=false;*root={};}
            else root=p.vacant(p.roots);if(!root)break;
            root->used=true;root->thread=thread;root->frame=frame;root->row_index=static_cast<std::uint32_t>(row-p.rows.data());
            root->manager=row->world+0xe15dc0;
            root->root=issued<RootAttributionId>(send(NativeSite::RootWorldLoaded,
                RootAttributionBegin{row->pass,row->iteration,row->context_publication,row->world_publication,
                    f.rcx,f.rcx,frame}));
        }break;
    case 0x79d240:
        forget_publications(f.rcx,0);
        if(row){row->setup={};row->context_birth={};row->context_reservation={};}
        if(row && chain.frames[0].return_pc==p.image+0x7a1ef2 &&
           chain.Find(0x7a1eb0) && chain.Find(0x7a1eb0)->return_pc==p.image+0x7b0261 &&
           chain.Find(0x7affc0) && chain.Find(0x7affc0)->return_pc==p.image+0x7b4270)
            row->setup=issued<ContextSetupId>(send(NativeSite::ContextSetupBegin,
                ContextSetupBegin{row->pass,row->row},row->frame));
        send(NativeSite::ContextConstructorBegin,ContextConstructBegin{row?std::optional(row->setup):std::nullopt,f.rcx});break;
    case 0x7a4220:send(NativeSite::ContextRetire,ContextRetire{f.rcx});forget_publications(f.rcx,0);break;
    case 0x636cb0:send(NativeSite::WorldRetireScene,WorldRetire{f.rcx,WorldRetireKind::SceneDisposer});forget_publications(f.rcx,1);break;
    case 0x5e3666:send(NativeSite::WorldRetireAlternative,WorldRetire{f.rdi,WorldRetireKind::AlternativeDisposer});forget_publications(f.rdi,1);break;
    case 0x5f2fb0:send(NativeSite::WorldRetireCommon,WorldRetire{f.rcx,WorldRetireKind::CommonOwnerDisposer});forget_publications(f.rcx,2);break;
    case 0x6385a0:send(NativeSite::ManagerRetireDispatch,ManagerRetire{f.rdx,ManagerRetireKind::DispatchOwner});forget_publications(f.rdx,3);break;
    case 0x5f31a0:send(NativeSite::ManagerRetireEarly,ManagerRetire{f.rcx,ManagerRetireKind::EarlyRecord});forget_publications(f.rcx,3);break;
    case 0x69103d:send(NativeSite::RRelocationBegin,RRelocationBegin{f.rcx});break;
    case 0x617573:mutate(f.rcx,SchemaMutationKind::Population,NativeSite::SchemaMutatePopulation);break;
    case 0x616a7e:mutate(f.rbx-8,SchemaMutationKind::Append,NativeSite::SchemaMutateAppend);break;
    case 0x5ffeb0:mutate(f.rcx-8,SchemaMutationKind::VectorGrowth,NativeSite::SchemaMutateVectorCallback);break;
    case 0x5e84e0:
        if(chain.Find(0x6167b0))mutate(f.rbx-8,SchemaMutationKind::Reorder,NativeSite::SchemaMutateReorder);break;
    case 0x7b41a7: {
        auto* native=chain.Find(0x7b40a0);if(!native)break;
        if(!row){row=p.vacant(p.rows);if(!row)break;row->frame=native->entry_rsp;row->thread=thread;
            row->pass=issued<RowPassId>(send(NativeSite::RowPassBegin,RowPassBegin{},row->frame));}
        // Replace before every fallible operation, including a different row.
        const auto previous_iteration=row->iteration;
        row->reservation={};row->iteration={};row->snapshot_count=0;row->row=f.rdi;
        row->context_scope_frame=0;row->world_setup_context=0;row->setup={};row->context_birth={};row->context_reservation={};row->world_birth={};row->world_reservation={};
        if(valid(previous_iteration))send(NativeSite::RowIterationEnd,RowIterationEnd{previous_iteration},row->frame);
        row->context_publication={};row->world_publication={};row->context=0;row->world=0;
        // Copy only host-owned identities BEFORE the actual Context MOV. The
        // current row map is never consulted by the post or an old child.
        if(auto* published=p.publication(row->row)) {
            row->context=published->context;row->world=published->world;
            row->context_publication=published->context_publication;
            row->world_publication=published->world_publication;
            for(const auto& b:p.bindings)if(b.used && b.world_publication==row->world_publication && valid(b.compile)) {
                if(row->snapshot_count==row->snapshot.size()){p.poisoned.store(true);break;}
                row->snapshot[row->snapshot_count++]={b.system,b.binding,b.compile};
            }
        }
        // Ledger snapshots the original published identities in the same event.
        const auto reply=send(NativeSite::RowReservationBegin,RowReservationBegin{row->pass,f.rdi},row->frame);
        row->reservation=issued<ReservationId>(reply);row->iteration=related<RowIterationId>(reply);
        break;}
    case 0x7b41b9:
        if(row){auto reservation=row->reservation;row->reservation={};
            const auto committed=issued<ContextPublicationId>(send(NativeSite::RowReservationComplete,
                RowReservationComplete{reservation,row->row,f.r8},row->frame));
            if(committed!=row->context_publication || f.r8!=row->context){row->context_publication={};row->world_publication={};row->snapshot_count=0;}}
        break;
    case 0x7b4290:
        if(row)row->world_setup_context=f.rdi==row->row?f.rdx:0;
        break;
    case 0x7a1ef2:
        if(row)row->context_birth=issued<ContextBirthId>(send(NativeSite::ContextBirthCaptured,
            ContextBirthCaptured{row->setup,f.rdi,f.rdi!=0}));
        break;
    case 0x7b0261:
        if(row){row->context=f.rbx;row->context_reservation=issued<ReservationId>(send(NativeSite::ContextPublicationStore,
            ContextPublicationBegin{row->setup,row->context_birth,row->row},row->frame));}break;
    case 0x7b427b:
        if(row){row->context_publication=issued<ContextPublicationId>(send(NativeSite::ContextPublicationComplete,
            ContextPublicationComplete{row->context_reservation,row->row,row->context,f.rdi==row->row && !(f.flags&0x40)},row->frame));row->context_reservation={};
            if(auto* pub=p.publication(row->row,true)){pub->context=row->context;pub->context_publication=row->context_publication;pub->world=0;pub->world_publication={};}}break;
    case 0x5f0230:
        forget_publications(f.rcx,2);
        if(row && chain.frames[0].return_pc==p.image+0x5ef413 &&
           chain.Find(0x618780) && chain.Find(0x618780)->return_pc==p.image+0x7a67e4 &&
           chain.Find(0x7a6590) && chain.Find(0x7a6590)->return_pc==p.image+0x7b429c && chain.Find(0x7a6590)->rdi==row->context && row->world_setup_context==row->context) {
            row->owner=f.rcx;row->world=f.rcx-0x928;row->manager=row->world+0xe15dc0;
            row->world_birth=issued<WorldBirthId>(send(NativeSite::WorldSetupBegin,
                WorldSetupBegin{row->context_publication,row->manager,row->owner}));
        } else {
            send(NativeSite::WorldSetupBegin,WorldSetupBegin{std::nullopt,f.rcx-0x928+0xe15dc0,f.rcx});
        }break;
    case 0x7a67e4:
        if(row && f.rax==row->world && f.rdi==row->context) {
            send(NativeSite::WorldBirthCaptured,WorldBirthCaptured{row->world_birth,f.rax,true});
            row->world_reservation=issued<ReservationId>(send(NativeSite::WorldPublicationStore,
                WorldPublicationBegin{row->world_birth,row->context_publication,f.rdi}));
        } else {
            send(NativeSite::WorldPublicationStore,WorldPublicationBegin{WorldBirthId{},ContextPublicationId{},f.rdi});
        }
        break;
    case 0x7a67f2:
        if(row){row->world_publication=issued<WorldPublicationId>(send(NativeSite::WorldPublicationComplete,
            WorldPublicationComplete{row->world_reservation,f.rdi,f.rax,f.rdi==row->context && f.rax==row->world}));row->world_reservation={};
            if(auto* pub=p.publication(row->row)){pub->world=row->world;pub->world_publication=row->world_publication;}}break;
    case 0x7b4405:
        if(row){send(NativeSite::RowIterationEnd,RowIterationEnd{row->iteration},row->frame);row->iteration={};row->reservation={};}break;
    case 0x7b4439:
        if(row){send(NativeSite::RowPassEnd,RowPassEnd{row->pass},row->frame);row->used=false;}break;
    case 0x7b44cf:
        send(NativeSite::RowContextClear,RowContextClear{f.rdi});
        if(auto* pub=p.publication(f.rdi))pub->used=false;
        break;
    default:break;
    }
    if(p.poisoned.load())CloseLocked(CloseReason::CapacityEviction);
    ReleaseSRWLockExclusive(&p.gate);
}
}
extern "C" void SF_SemanticNativeDispatch(SemanticAttribution::Native::SavedMachine* f,const void* xstate) noexcept {
    SemanticAttribution::DispatchNativeAttribution(f,xstate);
}
#ifdef SF_SEMANTIC_ATTRIBUTION_TESTING
#include "semantic-attribution/native_adapter_test_access.h"
namespace SemanticAttribution::testing {
NativeEventAdapter* NativeAdapterTestAccess::Construct(std::span<std::byte> storage,Ledger& ledger,
    std::span<const Native::ResidentSite> sites) noexcept {
    return NativeEventAdapter::Construct(storage,ledger,1,0x10000000,0x1000,sites);
}
bool NativeAdapterTestAccess::Publish(NativeEventAdapter& a) noexcept {return a.PublishTransport();}
void NativeAdapterTestAccess::LockCapture(NativeEventAdapter& a) noexcept {AcquireSRWLockExclusive(&a.impl_->gate);}
void NativeAdapterTestAccess::UnlockCapture(NativeEventAdapter& a) noexcept {ReleaseSRWLockExclusive(&a.impl_->gate);}
void NativeAdapterTestAccess::Poison(NativeEventAdapter& a) noexcept {a.impl_->poisoned.store(true,std::memory_order_release);}
bool NativeAdapterTestAccess::Poisoned(const NativeEventAdapter& a) noexcept {return a.impl_->poisoned.load(std::memory_order_acquire);}
bool NativeAdapterTestAccess::Published(const NativeEventAdapter& a) noexcept {return a.impl_->published.load(std::memory_order_acquire);}
bool NativeAdapterTestAccess::TransportIs(const NativeEventAdapter* a) noexcept {return transport.load(std::memory_order_acquire)==a;}
EventReply NativeAdapterTestAccess::SendPass(NativeEventAdapter& a) noexcept {
    const auto reply=a.Send(NativeSite::RowPassBegin,0x1000,Event{RowPassBegin{}});
    if(const auto* pass=std::get_if<RowPassId>(&reply.issued))
        a.Send(NativeSite::RowPassEnd,0x1000,Event{RowPassEnd{*pass}});
    return reply;
}
void NativeAdapterTestAccess::Capture(NativeEventAdapter& a,Native::SavedMachine& f) noexcept {a.Capture(f,nullptr);}
}
#endif
