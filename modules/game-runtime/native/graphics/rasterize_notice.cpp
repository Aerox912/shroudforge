#include "graphics/rasterize_notice.h"
#include "abi/notice_api.h"
#include "map/capture_game_map.h"
#include <windows.h>
#include <array>
#include <algorithm>

namespace OverlayRuntime {
std::int32_t RasterizeNotice(const std::uint8_t* input,std::uint32_t size,std::vector<std::uint8_t>& output) {
    if(!input || !size || size>SF_NOTICE_MAX_UTF8_BYTES)return 1;
    std::array<char,SF_NOTICE_MAX_UTF8_BYTES> bytes{};
    if(!MapRuntime::ReadMemory(reinterpret_cast<std::uintptr_t>(input),bytes.data(),size))return 1;
    for(std::uint32_t i=0;i<size;++i)if(static_cast<unsigned char>(bytes[i])<32 || bytes[i]==127)return 1;
    std::array<wchar_t,SF_NOTICE_MAX_UTF8_BYTES> text{};
    const auto count=MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,bytes.data(),int(size),text.data(),int(text.size()));
    if(!count)return 1;
    const auto dc=CreateCompatibleDC(nullptr);if(!dc)return 7;
    BITMAPINFO info{};info.bmiHeader.biSize=sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth=NoticeWidth;info.bmiHeader.biHeight=-int(NoticeHeight);
    info.bmiHeader.biPlanes=1;info.bmiHeader.biBitCount=32;info.bmiHeader.biCompression=BI_RGB;
    void* dib{};const auto bitmap=CreateDIBSection(dc,&info,DIB_RGB_COLORS,&dib,nullptr,0);
    const auto font=CreateFontW(-22,0,0,0,FW_NORMAL,FALSE,FALSE,FALSE,DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,ANTIALIASED_QUALITY,DEFAULT_PITCH,L"Segoe UI");
    if(!bitmap || !font || !dib){if(font)DeleteObject(font);if(bitmap)DeleteObject(bitmap);DeleteDC(dc);return 7;}
    const auto old_bitmap=SelectObject(dc,bitmap),old_font=SelectObject(dc,font);
    // Allocate before drawing, with scoped GDI cleanup even if allocation fails.
    try { output.resize(std::size_t(NoticeWidth)*NoticeHeight*4); }
    catch(...) {SelectObject(dc,old_font);SelectObject(dc,old_bitmap);DeleteObject(font);DeleteObject(bitmap);DeleteDC(dc);throw;}
    std::fill_n(static_cast<std::uint8_t*>(dib),output.size(),std::uint8_t{});
    SetBkMode(dc,TRANSPARENT);SetTextColor(dc,RGB(255,255,255));
    RECT area{16,8,int(NoticeWidth)-16,int(NoticeHeight)-8};
    const bool drawn=DrawTextW(dc,text.data(),count,&area,DT_SINGLELINE|DT_VCENTER|DT_CENTER|DT_END_ELLIPSIS|DT_NOPREFIX)!=0;
    const bool flushed=GdiFlush()!=0;
    // Grayscale glyph coverage is composited into an opaque dark panel. RGBA
    // is straight alpha, independent of the undefined alpha byte in GDI's DIB.
    const auto* source=static_cast<const std::uint8_t*>(dib);
    for(std::size_t i=0;i<output.size();i+=4) {
        const auto coverage=(std::max)({source[i],source[i+1],source[i+2]});
        output[i]=output[i+1]=output[i+2]=std::uint8_t(24+(231*coverage)/255);output[i+3]=255;
    }
    SelectObject(dc,old_font);SelectObject(dc,old_bitmap);DeleteObject(font);DeleteObject(bitmap);DeleteDC(dc);
    if(!drawn || !flushed){output.clear();return 7;}return 0;
}
}
