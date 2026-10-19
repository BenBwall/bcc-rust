// LLVM-C does not expose TargetOptions::UseInitArray. Set it before the
// ELF object lowering is initialized so GNU constructor/destructor attributes
// use the sections the Linux runtime executes, as Clang's driver requests.
#include "llvm-c/TargetMachine.h"
#include "llvm/Target/TargetMachine.h"

extern "C" void bcc_llvm_use_init_array(LLVMTargetMachineRef machine) {
    // LLVM-C represents LLVMTargetMachineRef as the owned TargetMachine
    // pointer. The caller keeps it live and has not started emission.
    reinterpret_cast<llvm::TargetMachine *>(machine)->Options.UseInitArray = true;
}
