// C++ exceptions thrown in a shared object and caught in the executable, iostream,
// std::string, and static construction order.
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

const std::vector<std::string> &lib_names();
int lib_parse(const std::string &text);

int main() {
    std::cout << "lib says: " << lib_names().at(0) << '\n';
    for (const char *input : {"42", "", "12x", "nope"}) {
        try {
            int value = lib_parse(input);
            std::cout << "parse '" << input << "' = " << value << '\n';
        } catch (const std::invalid_argument &e) {
            std::cout << "parse '" << input << "' threw invalid_argument\n";
        } catch (const std::runtime_error &e) {
            std::cout << "parse '" << input << "' threw runtime_error: " << e.what() << '\n';
        }
    }
    return 0;
}
