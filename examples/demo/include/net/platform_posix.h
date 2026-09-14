#pragma once

namespace net {
using native_handle = int;
constexpr native_handle invalid_handle = -1;
int posix_socket(int domain, int type, int protocol);
int posix_close(native_handle h);
}  // namespace net
