#include "llvm/IR/DerivedTypes.h"
#include "llvm/IR/Function.h"
#include "llvm/IR/PassManager.h"
#include "llvm/Pass.h"
#include "llvm/IR/Module.h"
#include "llvm/Passes/PassBuilder.h"
#include "llvm/Passes/PassPlugin.h"
#include "llvm/Support/raw_ostream.h"

using namespace llvm;

namespace {

struct HeapHookPass : public PassInfoMixin<HeapHookPass> {
    PreservedAnalyses run(Module& M, ModuleAnalysisManager& AM) {
        bool Modified = false;
        LLVMContext& Context = M.getContext();
        Type* VoidPtrTy = Type::getInt8Ty(Context)->getPointerTo();
        FunctionType* MallocTy = FunctionType::get(VoidPtrTy, {Type::getInt64Ty(Context)}, false);
        FunctionType* FreeTy = FunctionType::get(Type::getVoidTy(Context), {VoidPtrTy}, false);
        auto WrappedMalloc = M.getOrInsertFunction(
            "__wrapped_rust_malloc", MallocTy 
        );
        auto WrappedFree = M.getOrInsertFunction(
            "__wrapped_rust_free", FreeTy
        );
        
        for(auto &F: M) {
            for (auto& B: F) {
                // iterate instructions in basic blocks
                for(auto& I: B) {
                    if (auto* Op = dyn_cast<CallInst>(&I)) {
                        Function* CalledFunc = Op->getCalledFunction();
                        if (CalledFunc->getName().equals("malloc")) {
                            Op->setCalledFunction(WrappedMalloc);
                        } else if (CalledFunc->getName().equals("free")) {
                            Op->setCalledFunction(WrappedFree);
                        }
                    }
                }
            }
        }
        return Modified ? PreservedAnalyses::none() : PreservedAnalyses::all();
    };
};

}

extern "C" LLVM_ATTRIBUTE_WEAK ::llvm::PassPluginLibraryInfo
llvmGetPassPluginInfo() {
    return {
        .APIVersion = LLVM_PLUGIN_API_VERSION,
        .PluginName = "Skeleton pass",
        .PluginVersion = "v0.1",
        .RegisterPassBuilderCallbacks = [](PassBuilder &PB) {
            PB.registerPipelineStartEPCallback(
                [](ModulePassManager &MPM, OptimizationLevel Level) {
                    MPM.addPass(HeapHookPass());
                });
        }
    };
}
