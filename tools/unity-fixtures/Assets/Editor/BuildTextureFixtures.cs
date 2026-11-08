using System;
using System.IO;
using System.Linq;
using UnityEditor;
using UnityEngine;

// Builds one bundle per texture format from a generated image. Run with:
//   Unity -batchmode -quit -nographics -projectPath <this directory> -buildTarget Android
//         -executeMethod BuildTextureFixtures.Build -fixtureOutput <directory>
public static class BuildTextureFixtures
{
    const string Root = "Assets/GeneratedTextures";

    // Not a multiple of any block size, so the last blocks are cut at the edges of the image.
    const int Width = 90;
    const int Height = 70;

    // The importer leaves ETC uncompressed unless the size is a power of two.
    const int EtcWidth = 128;
    const int EtcHeight = 64;

    static readonly TextureImporterFormat[] Formats =
    {
        TextureImporterFormat.Alpha8,
        TextureImporterFormat.RGB24,
        TextureImporterFormat.RGBA32,
        TextureImporterFormat.RGB16,
        TextureImporterFormat.ETC_RGB4,
        TextureImporterFormat.ETC2_RGBA8,
        TextureImporterFormat.ASTC_4x4,
        TextureImporterFormat.ASTC_5x5,
        TextureImporterFormat.ASTC_6x6,
        TextureImporterFormat.ASTC_8x8,
        TextureImporterFormat.ASTC_10x10,
        TextureImporterFormat.ASTC_12x12,
    };

    public static void Build()
    {
        var args = Environment.GetCommandLineArgs();
        var output = args[Array.IndexOf(args, "-fixtureOutput") + 1];
        if (AssetDatabase.IsValidFolder(Root))
            AssetDatabase.DeleteAsset(Root);
        AssetDatabase.CreateFolder("Assets", "GeneratedTextures");

        foreach (var format in Formats)
        {
            var path = Root + "/" + format + ".png";
            var etc = format == TextureImporterFormat.ETC_RGB4 || format == TextureImporterFormat.ETC2_RGBA8;
            File.WriteAllBytes(path, etc ? Image(EtcWidth, EtcHeight) : Image(Width, Height));
            AssetDatabase.ImportAsset(path);
            var importer = (TextureImporter)AssetImporter.GetAtPath(path);
            importer.textureType = TextureImporterType.Default;
            importer.npotScale = TextureImporterNPOTScale.None;
            importer.mipmapEnabled = false;
            importer.alphaSource = TextureImporterAlphaSource.FromInput;
            importer.SetPlatformTextureSettings(new TextureImporterPlatformSettings
            {
                name = "Android",
                overridden = true,
                format = format,
                compressionQuality = 100,
            });
            importer.SaveAndReimport();
        }

        var bundles = Formats
            .Select(format => new AssetBundleBuild
            {
                assetBundleName = format.ToString(),
                assetNames = new[] { Root + "/" + format + ".png" },
            })
            .ToArray();
        Directory.CreateDirectory(output);
        if (BuildPipeline.BuildAssetBundles(output, bundles, BuildAssetBundleOptions.UncompressedAssetBundle, BuildTarget.Android) == null)
            throw new Exception("building the texture bundles failed");
    }

    // Smooth gradients, flat areas and noise, with an alpha channel that uses the whole range.
    static byte[] Image(int width, int height)
    {
        var pixels = new Color32[width * height];
        uint random = 20261108;
        for (var y = 0; y < height; y++)
        {
            for (var x = 0; x < width; x++)
            {
                random = random * 1664525 + 1013904223;
                var noise = (byte)(random >> 24);
                var flat = x / 18 % 2 == 0;
                pixels[y * width + x] = new Color32(
                    (byte)(x * 255 / (width - 1)),
                    (byte)(y * 255 / (height - 1)),
                    flat ? (byte)64 : noise,
                    y < height / 2 ? (byte)((x + y) * 255 / (width + height - 2)) : (byte)(x / 9 % 2 * 255));
            }
        }
        var texture = new Texture2D(width, height, TextureFormat.RGBA32, false);
        texture.SetPixels32(pixels);
        texture.Apply();
        return texture.EncodeToPNG();
    }
}
