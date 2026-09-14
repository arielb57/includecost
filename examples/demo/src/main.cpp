// Command-line client.
#include "app/app.h"
#include "util/log.h"

int main(int argc, char** argv) {
    app::Config cfg = app::parse_args(argc, argv);
    util::log("starting client");
    return app::run(cfg);
}
