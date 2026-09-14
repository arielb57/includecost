#pragma once

#include "util/strings.h"
#include <fmt/format.h>

namespace util {

enum class Level { debug, info, warning, error };

inline void log(Level level, const std::string& message) {
    static const char* names[] = {"debug", "info", "warning", "error"};
    fmt::print("[{}] {}\n", names[static_cast<int>(level)], message);
}

inline void log(const std::string& message) { log(Level::info, message); }

}  // namespace util
