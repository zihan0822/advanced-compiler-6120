#include "llvm/IR/DerivedTypes.h"
#include "llvm/IR/Function.h"
#include "llvm/IR/IRBuilder.h"
#include "llvm/IR/Instructions.h"
#include "llvm/IR/PassManager.h"
#include "llvm/Pass.h"
#include "llvm/IR/Module.h"
#include "llvm/IR/Constants.h"
#include "llvm/IR/GlobalVariable.h"
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
        FunctionType* MallocTy = FunctionType::get(VoidPtrTy, {Type::getInt64Ty(Context), VoidPtrTy}, false);
        FunctionType* FreeTy = FunctionType::get(Type::getVoidTy(Context), {VoidPtrTy, VoidPtrTy}, false);
        auto WrappedMalloc = M.getOrInsertFunction(
            "__wrapped_rust_malloc", MallocTy 
        );
        auto WrappedFree = M.getOrInsertFunction(
            "__wrapped_rust_free", FreeTy
        );
        auto HeapProfileDump = M.getOrInsertFunction(
            "__heap_alloc_profile", 
            FunctionType::get(Type::getVoidTy(Context), false)
        );
        for(auto &F: M) {
            auto CallSiteName = ConstantDataArray::getString(Context, F.getName(), true);
            GlobalVariable* GlobalNameVar = new GlobalVariable(
                M,
                CallSiteName->getType(),
                true,
                GlobalValue::PrivateLinkage,
                CallSiteName
            );
            
            for (auto& B: F) {
                // iterate instructions in basic blocks
                for(auto I = B.begin(); I != B.end(); ) {
                    auto Inst = I++;
                    if (auto* Op = dyn_cast<CallInst>(Inst)) {
                        IRBuilder<> Builder = IRBuilder(Op);
                        Function* CalledFunc = Op->getCalledFunction();
                        Constant* Zero = ConstantInt::get(Type::getInt32Ty(Context), 0);
                        Constant* Indices[] = {Zero, Zero};
                        Value* CallSiteName = ConstantExpr::getInBoundsGetElementPtr(GlobalNameVar->getValueType(), GlobalNameVar, Indices);
                        if (CalledFunc->getName().equals("malloc")) {
                            Value* RequestedSize = Op->getOperand(0);
                            auto NewCall = Builder.CreateCall(WrappedMalloc, {RequestedSize, CallSiteName});
                            Op->replaceAllUsesWith(NewCall);
                            Op->eraseFromParent(); 
                        } else if (CalledFunc->getName().equals("free")) {
                            Value* FreedPtr = Op->getOperand(0);
                            Builder.CreateCall(WrappedFree, {FreedPtr, CallSiteName});
                            Op->eraseFromParent(); 
                        }
                    }
                    if (auto* Op = dyn_cast<ReturnInst>(Inst)) {
                        if (F.getName().equals("main")) {
                            IRBuilder<> Builder = IRBuilder(Op);
                            Builder.CreateCall(HeapProfileDump);
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
        .PluginName = "HeapHook pass",
        .PluginVersion = "v0.1",
        .RegisterPassBuilderCallbacks = [](PassBuilder &PB) {
            PB.registerPipelineStartEPCallback(
                [](ModulePassManager &MPM, OptimizationLevel Level) {
                    MPM.addPass(HeapHookPass());
                });
        }
    };
}
