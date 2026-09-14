#ifndef JSON_JSON_H
#define JSON_JSON_H

#include "json/detail/parser.h"

namespace json {

inline Value parse(const std::string& text) {
    detail::Parser p{text, 0};
    return p.parse_value();
}

}  // namespace json

#endif
