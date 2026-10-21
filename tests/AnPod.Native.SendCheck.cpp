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

    int sentBytes = 0;
    calls = 0;
    const int counted = SendExactBytes(8, [&calls](int, int remaining) {
        ++calls;
        return remaining > 4 ? 4 : remaining;
    }, [&sentBytes](int sent) { sentBytes += sent; });
    assert(counted == 8 && calls == 2 && sentBytes == 8);
    sentBytes = 0; // a new capture session starts with an empty byte window
    assert(sentBytes == 0);

    calls = 0;
    int partialBytes = 0;
    const int timedOut = SendExactBytes(8, [&calls](int offset, int) {
        ++calls;
        return offset == 0 ? 4 : 0; // partial send followed by timeout
    }, [&partialBytes](int sent) { partialBytes += sent; });
    assert(timedOut == -1 && calls == 2 && partialBytes == 4); // count successful short sends before failure
    std::cout << "Native exact-send checks passed.\n";
}
