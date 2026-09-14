// Network server.
#include "app/app.h"
#include "net/socket.h"
#include <vector>

int main() {
    net::Socket listener = net::listen(8080);
    std::vector<net::Socket> clients;
    while (listener.valid()) {
        clients.push_back(listener.accept());
    }
    return 0;
}
