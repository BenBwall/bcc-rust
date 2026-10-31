// LLVM-C adds attributes one at a time. Attribute lists are immutable and
// uniqued in the context, so each LLVMAddAttributeAtIndex builds and keeps
// a new list as long as the highest index used: annotating every parameter
// of an n-parameter function or call that way keeps n lists of up to n sets,
// quadratic memory that the context frees only at disposal. This bridge
// builds the whole list once and merges it with the value's current one.
#include "llvm-c/Core.h"
#include "llvm/ADT/STLExtras.h"
#include "llvm/ADT/SmallVector.h"
#include "llvm/IR/Attributes.h"
#include "llvm/IR/Function.h"
#include "llvm/IR/InstrTypes.h"

#include <cstddef>
#include <utility>

// One attribute at an LLVM-C attribute index: 0 for the result, ~0 for the
// function, 1 + n for parameter n. Mirrors `IndexedAttribute` in api.rs.
struct BccIndexedAttribute {
    unsigned index;
    LLVMAttributeRef attribute;
};

extern "C" void bcc_llvm_add_attributes(LLVMValueRef value, int call,
                                        const BccIndexedAttribute *attributes,
                                        size_t count) {
    if (count == 0) {
        return;
    }
    llvm::Value *target = llvm::unwrap(value);
    llvm::LLVMContext &context = target->getContext();
    llvm::SmallVector<std::pair<unsigned, llvm::Attribute>, 16> pairs;
    pairs.reserve(count);
    for (size_t i = 0; i < count; ++i) {
        pairs.emplace_back(attributes[i].index, llvm::unwrap(attributes[i].attribute));
    }
    // AttributeList::get groups consecutive pairs by index.
    llvm::stable_sort(pairs, llvm::less_first());
    llvm::AttributeList added = llvm::AttributeList::get(context, pairs);
    if (call) {
        auto *site = llvm::cast<llvm::CallBase>(target);
        site->setAttributes(llvm::AttributeList::get(context, {site->getAttributes(), added}));
    } else {
        auto *function = llvm::cast<llvm::Function>(target);
        function->setAttributes(
            llvm::AttributeList::get(context, {function->getAttributes(), added}));
    }
}
