# The output adapter and optional MSI must come from this same native build.
# Pin bytes, not an untrusted adjacent manifest or a DLL's self-reported version.
file(SHA256 "${SOURCE_DLL}" digest)
file(WRITE "${PIN_HEADER}.tmp" "#pragma once\ninline constexpr char TingXianSourceSha256[] = \"${digest}\";\n")
execute_process(COMMAND "${CMAKE_COMMAND}" -E copy_if_different "${PIN_HEADER}.tmp" "${PIN_HEADER}"
  COMMAND_ERROR_IS_FATAL ANY)
