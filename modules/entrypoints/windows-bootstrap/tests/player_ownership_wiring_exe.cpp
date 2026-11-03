#include <windows.h>

extern "C" __declspec(dllimport) bool GateVerified();

int main() {
    return GateVerified() ? 0 : 1;
}
