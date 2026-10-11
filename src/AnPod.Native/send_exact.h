#pragma once

template <typename Send>
static int SendExactBytes(int length, Send sendBytes)
{
    int total = 0;
    while (total < length)
    {
        const int sent = sendBytes(total, length - total);
        if (sent <= 0) return -1;
        total += sent;
    }
    return total;
}
