import { execSync } from 'node:child_process';
import PACKAGE from '../package.json' with { type: 'json' };

// const target = `--target ${platform}-${arch}`
// const outputFileName = `xenomorph-${platform}-${arch}.vsix`;
const outputFileName = `xenomorph-${PACKAGE.version}.vsix`;

console.log(`Packaging extension "${outputFileName}"...`);
execSync(`vsce package --out ${outputFileName}`, {
    stdio: 'inherit',
});

console.log('✓ Extension packaged successfully!');

const shouldPublish = process.argv.includes('--publish');

if (shouldPublish) {
    console.log('Publishing extension to the marketplace...');
    execSync(`vsce publish --packagePath ${outputFileName}`, {
        stdio: 'inherit',
    });
    console.log('✓ Extension published successfully!');
}
