// LLVM-C does not expose these TargetOptions. Set them before the object
// lowering is initialized. UseInitArray makes GNU constructor/destructor
// attributes use the sections the Linux runtime executes, as Clang's driver
// requests; the section options are -ffunction-sections and -fdata-sections.
#include "llvm-c/TargetMachine.h"
#include "llvm/Target/TargetMachine.h"

extern "C" void bcc_llvm_use_init_array(LLVMTargetMachineRef machine) {
    // LLVM-C represents LLVMTargetMachineRef as the owned TargetMachine
    // pointer. The caller keeps it live and has not started emission.
    reinterpret_cast<llvm::TargetMachine *>(machine)->Options.UseInitArray = true;
}

extern "C" void bcc_llvm_use_sections(LLVMTargetMachineRef machine, bool functions, bool data) {
    // As above: the caller owns a live machine that has not started emission.
    auto &options = reinterpret_cast<llvm::TargetMachine *>(machine)->Options;
    options.FunctionSections = functions;
    options.DataSections = data;
}
