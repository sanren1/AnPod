#pragma once

template <typename Send, typename OnSent>
static int SendExactBytes(int length, Send sendBytes, OnSent onSent)
{
    int total = 0;
    while (total < length)
    {
        const int sent = sendBytes(total, length - total);
        if (sent <= 0) return -1;
        onSent(sent);
        total += sent;
    }
    return total;
}

template <typename Send>
static int SendExactBytes(int length, Send sendBytes)
{
    return SendExactBytes(length, sendBytes, [](int) {});
}
