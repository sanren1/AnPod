#include <cassert>
#include <iostream>
#include "../src/AnPod.Native/send_exact.h"

int main()
{
    int calls = 0;
    const int complete = SendExactBytes(8, [&calls](int, int remaining) {
        ++calls;
        return remaining > 4 ? 4 : remaining;
    });
    assert(complete == 8 && calls == 2);

    calls = 0;
    const int timedOut = SendExactBytes(8, [&calls](int offset, int) {
        ++calls;
        return offset == 0 ? 4 : 0; // partial send followed by timeout
    });
    assert(timedOut == -1 && calls == 2); // no next packet can be sent on this stream
    std::cout << "Native exact-send checks passed.\n";
}
