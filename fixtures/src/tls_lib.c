// Thread-local storage in a shared object: reached through __tls_get_addr
// (global-dynamic), the model every CS2 module uses.
__thread int lib_counter = 100;

int lib_bump(int by) {
    lib_counter += by;
    return lib_counter;
}
