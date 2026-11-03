#include "semantic-attribution/native_site_profile.h"
#include <cassert>
#include <cstring>
#include <cstdio>

using namespace SemanticAttribution::Native;
int main() {
    // These checks inspect server profile bytes as data. They never execute them.
    constexpr std::uintptr_t image=0x140000000ULL,relay=0x13f000000ULL;
    std::size_t sites{},instructions{},null_callbacks{};
    for(const auto& s:ArchivedServerProfile().sites) {
        RelayPlan plan{};
        assert(BuildRelay(s,image,relay,0x180000000ULL,plan));
        assert(plan.code_size<RelayPlan::CommonPointerOffset);
        assert(plan.bytes[plan.code_size-1]==0xc3);
        assert(plan.bytes[RelayPlan::UnwindOffset]==1);
        std::array<std::uint8_t,16> patch{};
        assert(EncodeSiteCall(s,image,relay,std::span(patch).first(s.expected_size)));
        assert(patch[0]==0xe8);
        for(auto c:ArchivedServerProfile().unchanged_operations)
            assert(s.rva+s.expected_size<=c.rva || c.rva+c.size<=s.rva);
        ++sites;instructions+=s.instruction_count;
    }
    for(const auto& d:ArchivedServerProfile().registration_catalog) {
        assert(d.descriptor_rva && d.descriptor_rva<ArchivedServerProfile().image_size);
        assert(!d.callback_rva || d.callback_rva<ArchivedServerProfile().image_size);
        if(!d.callback_rva)++null_callbacks;
    }
    assert(ArchivedServerProfile().registration_catalog.size()==817 && null_callbacks==197);
    // Own synthetic instruction fixtures exercise all displacement encodings.
    Site s{};s.expected_size=5;s.instruction_count=1;
    s.instructions[0]={{{0x48,0x8b,0x44,0x24,0x7f}},5,RelocationKind::OriginalStack,4,1,2,127,0};
    std::array<std::uint8_t,20> out{};std::size_t size{};
    assert(RelocateOriginal(s,image,relay,out,size));
    assert(size==8 && out[2]==0x84 && out[3]==0x24);
    std::int32_t d{};std::memcpy(&d,out.data()+4,4);assert(d==135);
    s.instructions[0]={{{0x48,0x8b,0x04,0x24}},4,RelocationKind::OriginalStack,0,0,2,0,0};
    assert(RelocateOriginal(s,image,relay,out,size));assert(size==5 && out[2]==0x44 && out[4]==8);
    s.instructions[0]={{{0x48,0x8d,0x0d,0,0,0,0}},7,RelocationKind::RipRelative,3,4,2,0,0x1000};
    assert(RelocateOriginal(s,image,relay,out,size));
    std::memcpy(&d,out.data()+3,4);assert(relay+7+d==image+0x1000);
    assert(!RelocateOriginal(s,image,0x700000000000ULL,out,size));
    assert(!EncodeSiteCall(s,image,0x700000000000ULL,std::span(out).first(5)));
    std::printf("SF-NA-30 relay data checks passed: %zu hooks, %zu instructions, %zu unchanged guards,817 registration tuples/197 literalnullcallbacks; synthetic relocation boundaries passed\n",sites,instructions,ArchivedServerProfile().unchanged_operations.size());
}
