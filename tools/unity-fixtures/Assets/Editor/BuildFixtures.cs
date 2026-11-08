using System;
using System.Collections.Generic;
using System.IO;
using UnityEditor;
using UnityEngine;

// Builds the fixture bundles. Run with:
//   Unity -batchmode -quit -nographics -projectPath <this directory> -buildTarget Android
//         -executeMethod BuildFixtures.Build -fixtureOutput <directory>
public static class BuildFixtures
{
    const string Root = "Assets/Generated";

    public static void Build()
    {
        var args = Environment.GetCommandLineArgs();
        var output = args[Array.IndexOf(args, "-fixtureOutput") + 1];
        CreateAssets();
        var bundles = new[]
        {
            new AssetBundleBuild
            {
                assetBundleName = "fixture_data",
                assetNames = new[]
                {
                    Root + "/data.asset", Root + "/small.asset", Root + "/large.asset", Root + "/blob.bytes",
                },
            },
            new AssetBundleBuild
            {
                assetBundleName = "fixture_scene",
                assetNames = new[] { Root + "/material.mat", Root + "/quad.asset", Root + "/quad.prefab" },
            },
        };
        var variants = new Dictionary<string, BuildAssetBundleOptions>
        {
            { "uncompressed", BuildAssetBundleOptions.UncompressedAssetBundle },
            { "lz4", BuildAssetBundleOptions.ChunkBasedCompression },
            { "lzma", BuildAssetBundleOptions.None },
        };
        foreach (var variant in variants)
        {
            var build = Path.Combine("Temp", "fixtures", variant.Key);
            Directory.CreateDirectory(build);
            var options = variant.Value | BuildAssetBundleOptions.DeterministicAssetBundle;
            if (BuildPipeline.BuildAssetBundles(build, bundles, options, BuildTarget.Android) == null)
                throw new Exception("building the " + variant.Key + " bundles failed");
            var target = Path.Combine(output, variant.Key);
            Directory.CreateDirectory(target);
            foreach (var bundle in bundles)
                File.Copy(Path.Combine(build, bundle.assetBundleName), Path.Combine(target, bundle.assetBundleName), true);
        }
    }

    static void CreateAssets()
    {
        if (AssetDatabase.IsValidFolder(Root))
            AssetDatabase.DeleteAsset(Root);
        AssetDatabase.CreateFolder("Assets", "Generated");

        var small = Texture(2, "small");
        var large = Texture(64, "large");

        // Not valid UTF-8, so the text field is written as a byte payload.
        File.WriteAllBytes(Root + "/blob.bytes", new byte[] { 0x66, 0x69, 0x78, 0xff, 0xfe, 0x00, 0x01 });
        AssetDatabase.ImportAsset(Root + "/blob.bytes");

        var material = new Material(Shader.Find("Unlit/Texture")) { mainTexture = large };
        AssetDatabase.CreateAsset(material, Root + "/material.mat");

        var mesh = new Mesh
        {
            name = "quad",
            vertices = new[] { new Vector3(0, 0, 0), new Vector3(1, 0, 0), new Vector3(0, 1, 0), new Vector3(1, 1, 0) },
            uv = new[] { new Vector2(0, 0), new Vector2(1, 0), new Vector2(0, 1), new Vector2(1, 1) },
            triangles = new[] { 0, 2, 1, 2, 3, 1 },
        };
        AssetDatabase.CreateAsset(mesh, Root + "/quad.asset");

        var go = new GameObject("quad");
        go.AddComponent<MeshFilter>().sharedMesh = mesh;
        go.AddComponent<MeshRenderer>().sharedMaterial = material;
        new GameObject("child").transform.SetParent(go.transform, false);
        PrefabUtility.SaveAsPrefabAsset(go, Root + "/quad.prefab");
        UnityEngine.Object.DestroyImmediate(go);

        var data = ScriptableObject.CreateInstance<FixtureData>();
        data.flag = true;
        data.signedByte = -5;
        data.unsignedByte = 250;
        data.shortValue = -12345;
        data.unsignedShort = 54321;
        data.intValue = -2000000000;
        data.unsignedInt = 4000000000;
        data.longValue = -9007199254740993;
        data.unsignedLong = 18446744073709551615;
        data.floatValue = 0.1f;
        // The bits a NaN reads back as after the asset is saved, so every build stores the same bytes.
        data.notANumber = BitConverter.Int32BitsToSingle(0x7fc00000);
        data.infinity = float.NegativeInfinity;
        data.doubleValue = 1.0 / 3.0;
        data.character = 'Z';
        data.text = "fixture いろは \"quoted\"\n";
        data.bytes = new byte[] { 0, 1, 2, 3, 254, 255 };
        data.numbers = new[] { 1, -1, int.MaxValue, int.MinValue };
        data.entries = new List<Entry>
        {
            new Entry { key = "a", value = 1, flags = new[] { true } },
            new Entry { key = "", value = -1, flags = new bool[0] },
        };
        data.position = new Vector3(1.5f, -2.25f, 1e-30f);
        data.color = new Color(1, 0.5f, 0.25f, 1);
        data.curve = AnimationCurve.EaseInOut(0, 0, 1, 1);
        data.texture = small;
        data.material = material;
        data.shapes = new List<Shape>
        {
            new Circle { label = "circle", radius = 2 },
            new Polygon
            {
                label = "polygon",
                points = new[] { new Vector2(0, 0), new Vector2(1, 0), new Vector2(0, 1) },
                inner = new Circle { label = "inner", radius = 0.5f },
            },
            null,
        };
        data.single = new Shape { label = "single" };
        AssetDatabase.CreateAsset(data, Root + "/data.asset");
        data.self = data;
        EditorUtility.SetDirty(data);
        AssetDatabase.SaveAssets();
    }

    static Texture2D Texture(int size, string name)
    {
        var texture = new Texture2D(size, size, TextureFormat.RGBA32, false) { name = name };
        var pixels = new Color32[size * size];
        for (var i = 0; i < pixels.Length; i++)
            pixels[i] = new Color32((byte)i, (byte)(i >> 8), (byte)(i * 7), 255);
        texture.SetPixels32(pixels);
        texture.Apply();
        AssetDatabase.CreateAsset(texture, Root + "/" + name + ".asset");
        return texture;
    }
}
