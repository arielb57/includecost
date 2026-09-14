// The application facade. Everything includes it; the question is what
// removing it would actually save.
#ifndef APP_APP_H
#define APP_APP_H

#include "util/log.h"
#include "net/socket.h"
#include "json/json.h"

namespace app {

struct Config {
    int port = 8080;
    bool verbose = false;
    json::Value settings;
};

Config parse_args(int argc, char** argv);
int run(const Config& cfg);

}  // namespace app

#endif  // APP_APP_H
