"""Run the intact M6 lowerer against the current retained producer in-place.
All modified files are backed up and restored, even if Cargo fails. Run alone
in the assigned M7 worktree. The patch/log are retained in the task scratch.
"""
from pathlib import Path
import argparse
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', required=True, type=Path,
                    help='Scratch directory for the log and instrumentation patch')
parser.add_argument('--wrapper', required=True,
                    help='Shared build wrapper required by the task rules')
args = parser.parse_args()
OUT = args.output.resolve()
OUT.mkdir(parents=True, exist_ok=True)
WRAPPER = args.wrapper
BASE = '9055bc01'

def old(path):
    return subprocess.check_output(['git', 'show', f'{BASE}:{path}']).decode()

def replace(text, before, after):
    assert before in text, before
    return text.replace(before, after, 1)

paths = subprocess.check_output(['git', 'diff', '--name-only', BASE, 'HEAD', '--', 'src']).decode().splitlines()
paths = [p for p in paths if not p.startswith(('src/backend/', 'src/builtins/')) and not p.endswith('facts/entities.rs')]
backups = {p: Path(p).read_bytes() for p in paths}
try:
    for p in paths:
        Path(p).write_text(old(p))
    p = Path('src/lowering/tests.rs')
    p.write_text(p.read_text().replace('    entities::compare_legacy(&context, &sema, target);\n', '').replace('mod entities;', 'pub(super) mod entities;'))
    # Compare at the production entry, not only the lower_with test helper.
    p = Path('src/lowering.rs')
    s = p.read_text()
    s = replace(s, '    let tables = Bump::new();', '''    #[cfg(test)]
    tests::entities::compare_legacy(context, sema, target);
    let tables = Bump::new();''')
    p.write_text(s)
    p = Path('src/lowering/tests/entities.rs')
    s = p.read_text().replace('pub(super) fn compare_legacy', 'pub(in crate::lowering) fn compare_legacy')
    s = s.replace('''        let fact = sema
            .entity(binding)
            .expect("every binding has entity policy");''', '''        let Some(fact) = sema.entity(binding) else {
            assert!(!data.needs_entity_policy());
            continue;
        };''')
    # Remove formula-only oracles; new checks below read the legacy state.
    start = s.index('        let returns_twice = sema')
    end = s.index('        let own = sema', start)
    s = s[:start] + '''        let old_open = unit.unprototyped_external(binding);
        let new_open = sema.unprototyped_external(binding).unwrap();
        if old_open != new_open {
            assert!(!old_open && new_open);
            assert!(sema.functions.iter().any(|f| f.binding.is_some_and(|nested|
                sema.scopes[sema.bindings[nested].scope].kind != crate::translation_phases::semantic_analysis::ScopeKind::File
                && unit.binding_identity(nested) == unit.binding_identity(binding)
                && sema.correspondence.entity_of(nested, sema.bindings[nested])
                    != sema.correspondence.entity_of(binding, *data))));
        }
''' + s[end:]
    start = s.index('    for function in sema.functions {')
    end = s.index('\n}\n', start)
    s = s[:start] + s[end:]
    s = s.replace('    FunctionEnd,\n', '')
    s = s.replace('''    if !sema.lowerable(context) {''', '''    if !sema.lowerable(context) || sema.correspondence.declarations.is_empty() {''')
    start = s.index('        let own = sema')
    end = s.index('        if matches!(sema.types.kind(data.ty)', start)
    s = s[:start] + s[end:]
    s = s.replace('                let entity = sema.entity(binding).unwrap();', '''                let Some(entity) = sema.entity(binding) else { continue; };''')
    # Don't compare the old all-binding snapshot with the new sparse snapshot.
    s = s.replace('#[test]\nfn m7_retained_policy_snapshot()', '#[test]\n#[ignore = "Storage shape changed; the current-tree snapshot is gated separately."]\nfn m7_retained_policy_snapshot()')
    extras = backups['src/lowering/tests/entities.rs'].decode()
    start = extras.index('#[test]\nfn m7_review_redeclaration_fixtures_keep_lowering()')
    s += '\n' + extras[start:]
    s += '''
#[test]
fn m7_review_nested_function_known_divergence() {
    for target in [Target::LinuxGnu, Target::LinuxMusl, Target::WindowsGnu, Target::WindowsMsvc] {
        drop(lower_for("int f(); int g(void){return f(42);} inline int outer(void){int f(int x){return x;} return f(1);}", target));
    }
}
'''
    p.write_text(s)
    p = Path('src/lowering/module_items.rs')
    s = p.read_text()
    s = replace(s, '        let defined_objects = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables);', '''        // Compare actual per-binding definition state and body selection.
        for function in sema.functions {
            if let Some(binding) = function.binding {
                let policy = sema.function_policy(binding).unwrap();
                assert_eq!(definitions.get(&binding) == Some(&DefinitionKind::Inline),
                    matches!(policy.body, crate::translation_phases::semantic_analysis::BodyEmission::InlineOnly
                        | crate::translation_phases::semantic_analysis::BodyEmission::PrivateInline));
            }
        }
        for (binding, data) in sema.bindings.iter().enumerate() {
            if let Some(entity) = sema.entity(binding) {
                if let Some(definition) = entity.definition {
                    assert_eq!(definitions.get(&definition.binding), Some(&definition.kind));
                }
            } else {
                assert!(!data.needs_entity_policy());
            }
        }
        let defined_objects = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables);''')
    s = replace(s, '            if let Err(error) = self.define_object(definition.binding, None, initializer) {', '''            let entity = self.sema.entity(definition.binding).unwrap();
            assert_eq!(entity.definition.map(|d| (d.binding, d.kind)), Some((definition.binding, definition.kind)));
            assert_eq!(entity.initializer.map(std::ptr::from_ref), initializer.map(std::ptr::from_ref));
            if let Err(error) = self.define_object(definition.binding, None, initializer) {''')
    # This is the actual flag produced after old symbol_attributes + asm logic.
    s = replace(s, '''        Ok(func)
    }

    /// An external declaration''', '''        assert_eq!(self.module.function(func).returns_twice,
            self.sema.entity(binding).unwrap().returns_twice);
        Ok(func)
    }

    /// An external declaration''')
    p.write_text(s)
    p = Path('src/lowering/statements.rs')
    s = p.read_text()
    s = replace(s, '''                    Some(if self.name == "main" {''', '''                    assert_eq!(self.name == "main",
                        self.unit.sema.function_policy(self.record.binding.unwrap()).unwrap().end
                            == crate::translation_phases::semantic_analysis::FunctionEnd::MainZero);
                    Some(if self.name == "main" {''')
    p.write_text(s)
    (OUT / 'round2-legacy-instrumentation.patch').write_bytes(subprocess.check_output(['git', 'diff', BASE, '--', 'src']))
    with (OUT / 'round2-legacy-all.log').open('w') as log:
        result = subprocess.run(['python3', WRAPPER, 'cargo', 'test', '--all-targets', '--no-fail-fast'], stdout=log, stderr=subprocess.STDOUT)
    raise SystemExit(result.returncode)
finally:
    for path, data in backups.items():
        Path(path).write_bytes(data)
