#pragma once

namespace net {
using native_handle = unsigned long long;
constexpr native_handle invalid_handle = ~0ull;
native_handle win_socket(int af, int type, int protocol);
int win_closesocket(native_handle h);
int win_startup(unsigned short version);
}  // namespace net
