#ifndef NET_SOCKET_H
#define NET_SOCKET_H

#include "util/log.h"
#ifdef _WIN32
#include "net/platform_win.h"
#else
#include "net/platform_posix.h"
#endif

namespace net {

class Socket {
public:
    explicit Socket(native_handle h) : handle_(h) {}
    bool valid() const { return handle_ != invalid_handle; }
    Socket accept();
    long send(const char* data, unsigned long size);
    long receive(char* data, unsigned long size);

private:
    native_handle handle_;
};

Socket listen(int port);

}  // namespace net

#endif
