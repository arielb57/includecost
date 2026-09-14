#ifndef UTIL_STRINGS_H
#define UTIL_STRINGS_H

#include <string>

namespace util {

inline std::string trim(const std::string& s) {
    const char* ws = " \t\r\n";
    auto begin = s.find_first_not_of(ws);
    if (begin == std::string::npos) return {};
    auto end = s.find_last_not_of(ws);
    return s.substr(begin, end - begin + 1);
}

inline bool starts_with(const std::string& s, const std::string& prefix) {
    return s.compare(0, prefix.size(), prefix) == 0;
}

}  // namespace util

#endif
