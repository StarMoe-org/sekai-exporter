using System;
using System.Collections.Generic;
using UnityEngine;

// One field of every kind the object document specification distinguishes.
public class FixtureData : ScriptableObject
{
    public bool flag;
    public sbyte signedByte;
    public byte unsignedByte;
    public short shortValue;
    public ushort unsignedShort;
    public int intValue;
    public uint unsignedInt;
    public long longValue;
    public ulong unsignedLong;
    public float floatValue;
    public float notANumber;
    public float infinity;
    public double doubleValue;
    public char character;
    public string text;
    public byte[] bytes;
    public int[] numbers;
    public List<Entry> entries;
    public Vector3 position;
    public Color color;
    public AnimationCurve curve;
    public Texture2D texture;
    public Material material;
    public FixtureData self;
    [SerializeReference] public List<Shape> shapes;
    [SerializeReference] public Shape single;
    [SerializeReference] public Shape none;
}

[Serializable]
public struct Entry
{
    public string key;
    public int value;
    public bool[] flags;
}

[Serializable]
public class Shape
{
    public string label;
}

[Serializable]
public class Circle : Shape
{
    public float radius;
}

[Serializable]
public class Polygon : Shape
{
    public Vector2[] points;
    [SerializeReference] public Shape inner;
}
