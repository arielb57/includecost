#ifndef JSON_DETAIL_PARSER_H
#define JSON_DETAIL_PARSER_H

#include <map>
#include <memory>
#include <string>
#include <vector>

// Only app.h reaches this file, through json.h, so every byte of it is
// really carried by that one include chain.
namespace json {

struct Value {
    enum class Kind { null, boolean, number, string, array, object };
    Kind kind = Kind::null;
    bool boolean = false;
    double number = 0;
    std::string string;
    std::vector<Value> array;
    std::map<std::string, Value> object;
};

namespace detail {

struct Parser {
    const std::string& text;
    std::size_t pos;

    void skip_ws() {
        while (pos < text.size() && (text[pos] == ' ' || text[pos] == '\n' || text[pos] == '\t' || text[pos] == '\r')) {
            ++pos;
        }
    }

    bool eat(const char* literal) {
        std::size_t n = std::char_traits<char>::length(literal);
        if (text.compare(pos, n, literal) != 0) return false;
        pos += n;
        return true;
    }

    Value parse_value() {
        skip_ws();
        Value v;
        if (eat("null")) return v;
        if (eat("true")) { v.kind = Value::Kind::boolean; v.boolean = true; return v; }
        if (eat("false")) { v.kind = Value::Kind::boolean; return v; }
        if (pos < text.size() && text[pos] == '"') { v.kind = Value::Kind::string; v.string = parse_string(); return v; }
        if (pos < text.size() && text[pos] == '[') return parse_array();
        if (pos < text.size() && text[pos] == '{') return parse_object();
        v.kind = Value::Kind::number;
        v.number = parse_number();
        return v;
    }

    std::string parse_string() {
        std::string out;
        ++pos;
        while (pos < text.size() && text[pos] != '"') {
            if (text[pos] == '\\' && pos + 1 < text.size()) {
                ++pos;
                switch (text[pos]) {
                    case 'n': out.push_back('\n'); break;
                    case 't': out.push_back('\t'); break;
                    case 'r': out.push_back('\r'); break;
                    default: out.push_back(text[pos]); break;
                }
            } else {
                out.push_back(text[pos]);
            }
            ++pos;
        }
        ++pos;
        return out;
    }

    double parse_number() {
        std::size_t start = pos;
        while (pos < text.size() && (std::isdigit(static_cast<unsigned char>(text[pos])) || text[pos] == '-' || text[pos] == '.' || text[pos] == 'e')) {
            ++pos;
        }
        return std::stod(text.substr(start, pos - start));
    }

    Value parse_array() {
        Value v;
        v.kind = Value::Kind::array;
        ++pos;
        skip_ws();
        if (eat("]")) return v;
        do {
            v.array.push_back(parse_value());
            skip_ws();
        } while (eat(","));
        eat("]");
        return v;
    }

    Value parse_object() {
        Value v;
        v.kind = Value::Kind::object;
        ++pos;
        skip_ws();
        if (eat("}")) return v;
        do {
            skip_ws();
            std::string key = parse_string();
            skip_ws();
            eat(":");
            v.object[key] = parse_value();
            skip_ws();
        } while (eat(","));
        eat("}");
        return v;
    }
};

}  // namespace detail
}  // namespace json

#endif
