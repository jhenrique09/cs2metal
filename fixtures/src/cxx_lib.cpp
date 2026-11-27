// A C++ shared object that throws across its boundary and has static constructors.
#include <stdexcept>
#include <string>
#include <vector>

namespace {
struct Registry {
    std::vector<std::string> names;
    Registry() { names.push_back("constructed before main"); }
};
Registry registry;
}  // namespace

const std::vector<std::string> &lib_names() { return registry.names; }

int lib_parse(const std::string &text) {
    if (text.empty()) throw std::invalid_argument("empty input");
    size_t used = 0;
    int value = std::stoi(text, &used);  // throws std::invalid_argument itself
    if (used != text.size()) throw std::runtime_error("trailing characters in '" + text + "'");
    return value;
}
