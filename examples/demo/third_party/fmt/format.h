// A stand-in for a large formatting library.
#ifndef FMT_FORMAT_H_
#define FMT_FORMAT_H_

#include <cstdio>
#include <string>

namespace fmt {

namespace detail {

inline void append_arg(std::string& out, const std::string& v) { out += v; }
inline void append_arg(std::string& out, const char* v) { out += v; }
inline void append_arg(std::string& out, int v) { out += std::to_string(v); }
inline void append_arg(std::string& out, long v) { out += std::to_string(v); }
inline void append_arg(std::string& out, long long v) { out += std::to_string(v); }
inline void append_arg(std::string& out, unsigned v) { out += std::to_string(v); }
inline void append_arg(std::string& out, unsigned long v) { out += std::to_string(v); }
inline void append_arg(std::string& out, double v) { out += std::to_string(v); }
inline void append_arg(std::string& out, bool v) { out += v ? "true" : "false"; }
inline void append_arg(std::string& out, char v) { out.push_back(v); }

inline void format_to(std::string& out, const char* f) {
    for (; *f; ++f) {
        if (f[0] == '{' && f[1] == '{') { out.push_back('{'); ++f; continue; }
        if (f[0] == '}' && f[1] == '}') { out.push_back('}'); ++f; continue; }
        out.push_back(*f);
    }
}

template <typename T, typename... Rest>
void format_to(std::string& out, const char* f, const T& value, const Rest&... rest) {
    for (; *f; ++f) {
        if (f[0] == '{' && f[1] == '{') { out.push_back('{'); ++f; continue; }
        if (f[0] == '{' && f[1] == '}') {
            append_arg(out, value);
            format_to(out, f + 2, rest...);
            return;
        }
        out.push_back(*f);
    }
}

}  // namespace detail

template <typename... Args>
std::string format(const char* f, const Args&... args) {
    std::string out;
    detail::format_to(out, f, args...);
    return out;
}

template <typename... Args>
void print(const char* f, const Args&... args) {
    std::string s = format(f, args...);
    std::fwrite(s.data(), 1, s.size(), stdout);
}

template <typename... Args>
void print(std::FILE* file, const char* f, const Args&... args) {
    std::string s = format(f, args...);
    std::fwrite(s.data(), 1, s.size(), file);
}

}  // namespace fmt

#endif  // FMT_FORMAT_H_
